# frozen_string_literal: true

# Stream values at every stage the pinned Ruby implementation
# (ukiryu/ukiryu@ab6df353) shows them, for byte payloads written by a real
# child process through the bash executor:
#
#   1. Executor.execute               -> Execution::Result (raw stdout/stderr)
#   2. Execution::Output / Result readers, to_h, to_json
#   3. Models::SuccessResponse.from_result -> Models::OutputInfo
#   4. CliCommands::ResponseFormatter#output_response (raw, yaml, json;
#      with and without an output file)
#
# Usage: ruby capture_stream_values.rb OUT.json
require 'json'
require 'rbconfig'
require 'stringio'
require 'tmpdir'
require 'yaml'
require 'ukiryu'

E = Ukiryu::Executor

PAYLOADS = {
  'empty' => '',
  'newline' => "\n",
  'crlf_lines' => "a\r\nb\r\n",
  'whitespace_only' => " \t\n",
  'nul_middle' => "a\x00b",
  'nul_edges' => "\x00 a \x00",
  'utf8_text' => "héllo ✓ \u{1d11e}\n",
  'utf8_edges' => " é \n",
  'all_bytes' => (0..255).to_a.pack('C*'),
  'invalid_middle' => "a\xffb",
  'invalid_after_space' => " \xffa",
  'invalid_truncated_end' => "ok\xe2\x82",
  'invalid_before_space' => "ok\xff \n",
  'invalid_mixed' => "\xe2\x82A\xc0\x80\xed\xa0\x80\xf4\x90\x80\x80\xf0\x9f\x98B\x80\xfe\xc3\xa9",
  'invalid_lines' => "x\xff\ny\r\nz",
  'invalid_after_newline' => "\n\x80"
}.transform_values { |v| v.b }.freeze

NEEDLES = ['a', "\x00", "é", "\n", "b\r\n"].freeze
REGEXES = [/a/, /\A\z/].freeze

FIXED_START = Time.at(1_700_000_000, 250_000, :usec, in: '+05:00')
FIXED_FINISH = Time.at(1_700_000_001, 500_000, :usec, in: '+05:00')

SCRIPT = 'printf "$1"; printf "$2" >&2; exit "$3"'

def fmt(bytes)
  bytes.unpack('C*').map { |b| format('\\x%02x', b) }.join
end

def hex(s)
  s.b.unpack1('H*')
end

def err(e)
  { 'error' => { 'class' => e.class.name, 'message_hex' => hex(e.message) } }
end

def attempt
  yield
rescue StandardError => e
  err(e)
end

def text(s)
  { 'hex' => hex(s), 'encoding' => s.encoding.name, 'valid' => s.valid_encoding? }
end

# The run command's formatter, called directly (say writes to $stdout).
def formatter
  Ukiryu::CliCommands::RunCommand.new({})
end

Config = Struct.new(:use_color) do
  def colors_disabled?
    use_color == false
  end
end

def capture_writes
  out = StringIO.new(''.b)
  er = StringIO.new(''.b)
  saved = [$stdout, $stderr]
  $stdout = out
  $stderr = er
  begin
    yield
  ensure
    $stdout, $stderr = saved
  end
  { 'stdout_hex' => hex(out.string), 'stderr_hex' => hex(er.string) }
end

def render(response)
  rows = {}
  Dir.mktmpdir do |dir|
    %w[raw yaml json].each do |format|
      config = Config.new(false)
      rows[format] = attempt { capture_writes { formatter.output_response(response, format.to_sym, nil, config) } }
      path = File.join(dir, "out-#{format}")
      with_file = attempt do
        w = capture_writes { formatter.output_response(response, format.to_sym, path, config) }
        w['file_hex'] = File.exist?(path) ? hex(File.binread(path)) : nil
        w['stdout_hex'] = hex(w['stdout_hex'].then { |h| [h].pack('H*') }.sub(path, '<FILE>'))
        w
      end
      rows["#{format}_file"] = with_file
    end
  end
  # The YAML documents as Psych loads them: the port compares loaded values
  # (Yeptris emits its own quoting and line breaks); the bytes stay recorded.
  # The bytes depend on the host's libyaml: 0.2.5 writes a NUL unquoted, and
  # Psych then cannot load the document (recorded as the error).
  rows['yaml']['psych_load_json'] = psych_load(rows['yaml']['stdout_hex'])
  rows['yaml_file']['psych_load_json'] = psych_load(rows['yaml_file']['file_hex'])
  rows
end

def psych_load(yaml_hex)
  attempt { JSON.generate(YAML.safe_load([yaml_hex].pack('H*').force_encoding('UTF-8'))) }
end

def stage_rows(result)
  # Result#output is the stripped stdout; the Output object is @output.
  o = result.instance_variable_get(:@output)
  row = {
    'raw_stdout' => text(result.stdout),
    'raw_stderr' => text(result.stderr),
    'output' => attempt { text(result.output) },
    'error_output' => attempt { text(result.error_output) },
    'stdout_lines' => attempt { result.stdout_lines.map { |l| hex(l) } },
    'stderr_lines' => attempt { result.stderr_lines.map { |l| hex(l) } },
    'stdout_contains' => NEEDLES.map { |n| attempt { result.stdout_contains?(n) } },
    'stderr_contains' => NEEDLES.map { |n| attempt { result.stderr_contains?(n) } },
    'stdout_matches' => REGEXES.map { |r| attempt { result.stdout_contains?(r) } },
    'stderr_matches' => REGEXES.map { |r| attempt { result.stderr_contains?(r) } },
    'stdout_empty' => attempt { o.stdout_empty? },
    'stderr_empty' => attempt { o.stderr_empty? },
    'stdout_length' => o.stdout_length,
    'stderr_length' => o.stderr_length,
    'output_to_s' => o.to_s,
    'output_to_h_json' => attempt { hex(JSON.generate(o.to_h)) }
  }
  fixed = Ukiryu::Execution::Result.new(
    command_info: result.command_info,
    output: o,
    metadata: Ukiryu::Execution::ExecutionMetadata.new(started_at: FIXED_START, finished_at: FIXED_FINISH,
                                                       timeout: 30)
  )
  row['result_to_json'] = attempt { hex(fixed.to_json) }
  response = attempt { Ukiryu::Models::SuccessResponse.from_result(fixed, {}, nil) }
  if response.is_a?(Hash)
    row['from_result'] = response
  else
    row['from_result'] = { 'stdout' => text(response.output.stdout), 'stderr' => text(response.output.stderr) }
    row['render'] = render(response)
  end
  row
end

def run_case(id, out, err_bytes, status)
  row = { 'id' => id, 'stdout_payload_hex' => hex(out), 'stderr_payload_hex' => hex(err_bytes),
          'status' => status, 'arguments' => ['-c', SCRIPT, 'emit', fmt(out), fmt(err_bytes), status.to_s] }
  begin
    r = E.execute('bash', row['arguments'], env: {}, shell: :bash, timeout: 30)
  rescue StandardError => e
    row['outcome'] = 'raise'
    row.merge!(err(e))
  end
  if r
    row['outcome'] = 'result'
    row['full_command'] = r.command_info.full_command
    row.merge!(stage_rows(r))
  end
  raise "#{id}: stdout bytes differ" if row['raw_stdout'] && row['raw_stdout']['hex'] != hex(out)
  raise "#{id}: stderr bytes differ" if row['raw_stderr'] && row['raw_stderr']['hex'] != hex(err_bytes)

  row
end

rows = []
PAYLOADS.each do |name, bytes|
  rows << run_case("stdout_#{name}", bytes, ''.b, 0)
  rows << run_case("stderr_#{name}", ''.b, bytes, 0)
end
rows << run_case('both_source_fixture', "A\x00\xff\x80B".b, "C\xfeD".b, 0)
rows << run_case('both_valid_nul', "A\x00B\n".b, "C\x00D\n".b, 0)
rows << run_case('failed_valid', "o\x00k\n".b, "bad\n".b, 3)
rows << run_case('failed_stdout_invalid', "a\xffb".b, "bad\n".b, 3)
rows << run_case('failed_stdout_invalid_after_space', " \xffa".b, ''.b, 3)
rows << run_case('failed_stderr_invalid', "ok\n".b, "e\xfe".b, 3)

doc = {
  'oracle' => {
    'repo' => 'ukiryu/ukiryu',
    'sha' => 'ab6df353c6dc2d7faf7817a86aafcafa82579896',
    'files' => %w[lib/ukiryu/executor.rb lib/ukiryu/execution/output.rb lib/ukiryu/execution/result.rb
                  lib/ukiryu/models/success_response.rb lib/ukiryu/models/output_info.rb
                  lib/ukiryu/cli_commands/response_formatter.rb],
    'lutaml_model' => Gem.loaded_specs['lutaml-model']&.version&.to_s,
    'ruby' => RUBY_DESCRIPTION,
    'host' => RbConfig::CONFIG['host_os'],
    'bash' => `bash --version`.lines.first.strip,
    'libyaml' => Psych::LIBYAML_VERSION,
    'capture_script' => 'ukiryu/tests/stream_values_data/capture_stream_values.rb'
  },
  'needles_hex' => NEEDLES.map { |n| hex(n) },
  'regexes' => REGEXES.map(&:source),
  'fixed_metadata' => {
    'started_at' => { 'seconds' => FIXED_START.to_i, 'nanos' => FIXED_START.nsec, 'utc_offset' => FIXED_START.utc_offset },
    'finished_at' => { 'seconds' => FIXED_FINISH.to_i, 'nanos' => FIXED_FINISH.nsec,
                       'utc_offset' => FIXED_FINISH.utc_offset },
    'timeout' => 30
  },
  'rows' => rows
}
File.write(ARGV.fetch(0), "#{JSON.pretty_generate(doc)}\n")
puts "#{rows.size} rows"
