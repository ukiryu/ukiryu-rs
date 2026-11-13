# frozen_string_literal: true

# K20-RESULTS oracle capture. Runs the ORIGINAL pinned Ruby code
# (ukiryu/ukiryu ab6df353c6dc2d7faf7817a86aafcafa82579896) and writes JSON golden
# rows into the directory given as ARGV[0]. Run from the oracle checkout's
# bundle with UKIRYU_ORACLE_DIR pointing at it, e.g.
#   UKIRYU_ORACLE_DIR=$ORACLE bundle exec ruby -I$ORACLE/lib capture.rb OUTDIR
require 'json'
require 'ukiryu'
require 'ukiryu/response/base'

OUT = ARGV.fetch(0)
SHA = `git -C #{ENV.fetch('UKIRYU_ORACLE_DIR')} rev-parse HEAD`.strip

def time_json(t)
  return nil if t.nil?

  { 'seconds' => t.to_i, 'nanos' => t.nsec, 'utc_offset' => t.utc_offset, 'utc' => t.utc? }
end

def num_json(n)
  case n
  when nil then nil
  when Integer then { 'int' => n }
  when Float then { 'float' => n.to_s }
  end
end

def write(name, rows, source)
  File.open(File.join(OUT, "#{name}.jsonl"), 'w') do |f|
    rows.each do |r|
      f.puts(JSON.generate({ 'source' => source }.merge(r)))
    end
  end
  warn "#{name}: #{rows.size}"
end

# Executor runs happen BEFORE Ukiryu::Execution is required: the pinned
# executor's `autoload :Execution` inside Ukiryu::Executor raises NameError
# when Ukiryu::Execution was loaded first (see README.md).
# Real process runs through the pinned executor (Linux bash): message, class, fields.
real = []
[[0.2, 'sleep', ['5']], [1, 'sleep', ['5']]].each do |to, exe, args|
  Ukiryu::Executor.execute(exe, args, shell: :bash, timeout: to)
rescue StandardError => e
  real << { 'case' => 'timeout', 'input' => { 'executable' => exe, 'args' => args, 'timeout' => num_json(to) },
            'expect' => { 'class' => e.class.name, 'message' => e.message,
                          'timeout_field' => e.respond_to?(:timeout) ? num_json(e.timeout) : 'absent' } }
end
['sh', ['-c', 'echo out; echo err >&2; exit 42']].then do |exe, args|
  Ukiryu::Executor.execute(exe, args, shell: :bash, timeout: 10)
rescue StandardError => e
  real << { 'case' => 'nonzero', 'input' => { 'executable' => exe, 'args' => args },
            'expect' => { 'class' => e.class.name, 'message' => e.message,
                          'has_result' => e.respond_to?(:result) } }
end
r = Ukiryu::Executor.execute('sh', ['-c', 'printf "a\nb\n"; exit 3'], shell: :bash, timeout: 10, allow_failure: true,
                             tool_name: 'sh', command_name: 'run')
real << { 'case' => 'allow_failure', 'input' => { 'executable' => 'sh', 'args' => ['-c', "printf \"a\nb\n\"; exit 3"] },
          'expect' => { 'class' => r.class.name, 'status' => r.status, 'stdout' => r.stdout, 'command' => r.command,
                        'success' => r.success?, 'shell' => r.command_info.shell.to_s, 'duration_class' => r.duration.class.name } }
write('executor_real', real, "lib/ukiryu/executor.rb Executor.execute (real processes, #{RUBY_PLATFORM}, bash)")

require 'ukiryu/execution'

# ---------------------------------------------------------------- Output
TEXTS = ['', 'hello', "hello\n", "  padded  \n", "a\r\nb\r\n", "a\nb\n\n\n", "\n\nx\n", "\0x\0",
         "\v\fx\t", "üñí\n", "line1\rline2", "\r\n", "a\n\r\n", "  \n  ", "tab\there\n", "quote\"back\\slash",
         "ctl\u0001\u001f\u007f", "日本語テキスト"].freeze
STATUSES = [0, 1, 2, 42, 137, 255, -1, 128].freeze
STR_PATTERNS = ['', 'hello', "b\r", 'ü', 'x', "\n", 'missing'].freeze
# [ruby source, rust regex equivalent]; Ruby ^/$ are line anchors, so the
# Rust form adds (?m). The translation is hand-written and listed here.
RE_PATTERNS = [['h.llo', 'h.llo'], ['\d+', '\d+'], ['^a$', '(?m)^a$'], ['x\z', 'x\z'], ['[[:space:]]{2}', '[[:space:]]{2}']].freeze

output_rows = []
TEXTS.each_with_index do |out, i|
  err = TEXTS[(i + 5) % TEXTS.size]
  status = STATUSES[i % STATUSES.size]
  o = Ukiryu::Execution::Output.new(stdout: out, stderr: err, exit_status: status)
  output_rows << {
    'input' => { 'stdout' => out, 'stderr' => err, 'exit_status' => status },
    'expect' => {
      'stdout' => o.stdout, 'stderr' => o.stderr,
      'stdout_lines' => o.stdout_lines, 'stderr_lines' => o.stderr_lines,
      'stdout_empty' => o.stdout_empty?, 'stderr_empty' => o.stderr_empty?,
      'stdout_length' => o.stdout_length, 'stderr_length' => o.stderr_length,
      'success' => o.success?, 'failure' => o.failure?,
      'to_h_json' => JSON.generate(o.to_h), 'to_s' => o.to_s, 'inspect' => o.inspect,
      'stdout_contains' => STR_PATTERNS.map { |p| o.stdout_contains?(p) },
      'stderr_contains' => STR_PATTERNS.map { |p| o.stderr_contains?(p) },
      'stdout_matches' => RE_PATTERNS.map { |rb, _| o.stdout_contains?(Regexp.new(rb)) },
      'stderr_matches' => RE_PATTERNS.map { |rb, _| o.stderr_contains?(Regexp.new(rb)) }
    }
  }
end
output_rows.unshift({ 'patterns' => STR_PATTERNS, 'regexes' => RE_PATTERNS })
write('output', output_rows, 'lib/ukiryu/execution/output.rb Ukiryu::Execution::Output')

# ---------------------------------------------------------------- Metadata
BASES = [
  Time.at(1_768_471_200, 0, :nsec, in: '+05:00'),
  Time.at(1_768_471_200, 250_000_000, :nsec).utc,
  Time.at(-1, 999_999_999, :nsec, in: '-03:30'),
  Time.at(1_700_000_000, 123, :nsec, in: '+00:00')
].freeze
DUR_NS = [0, 1, 500_000, 123_456_789, 999_999_999, 1_000_000_000, 1_500_000_000, 59_999_999_999,
          60_000_000_000, 61_250_000_000, 125_990_000_000, 3_600_500_000_000, -500_000_000, 999_994_999,
          9_999_000, 1_005_000, 2_675_000, 59_950_000_000, 119_960_000_000].freeze
TIMEOUTS = [nil, 0, 1, 30, 0.5, 1.0, 90].freeze

def meta_expect(m)
  h = m.to_h
  {
    'duration' => m.duration&.to_s, 'duration_seconds' => m.duration_seconds&.to_s,
    'duration_milliseconds' => m.duration_milliseconds&.to_s,
    'timed_out' => m.timed_out?, 'formatted_duration' => m.formatted_duration,
    'to_s' => m.to_s, 'inspect' => m.inspect, 'to_h_json' => JSON.generate(h),
    'started_iso8601' => m.started_at.respond_to?(:iso8601) ? m.started_at.iso8601 : nil,
    'finished_iso8601' => m.finished_at.respond_to?(:iso8601) ? m.finished_at.iso8601 : nil
  }
end

meta_rows = []
DUR_NS.each_with_index do |ns, i|
  base = BASES[i % BASES.size]
  fin_total = base.to_i * 1_000_000_000 + base.nsec + ns
  fin = Time.at(fin_total.div(1_000_000_000), fin_total % 1_000_000_000, :nsec, in: base.utc? ? 'UTC' : base.utc_offset)
  fin = fin.utc if base.utc?
  TIMEOUTS.each do |to|
    m = Ukiryu::Execution::ExecutionMetadata.new(started_at: base, finished_at: fin, timeout: to)
    meta_rows << { 'input' => { 'started_at' => time_json(base), 'finished_at' => time_json(fin), 'timeout' => num_json(to) },
                   'expect' => meta_expect(m) }
  end
end
[[nil, BASES[0]], [BASES[0], nil], [nil, nil]].each do |s, f|
  [nil, 5].each do |to|
    m = Ukiryu::Execution::ExecutionMetadata.new(started_at: s, finished_at: f, timeout: to)
    meta_rows << { 'input' => { 'started_at' => time_json(s), 'finished_at' => time_json(f), 'timeout' => num_json(to) },
                   'expect' => meta_expect(m) }
  end
end
write('metadata', meta_rows, 'lib/ukiryu/execution/metadata.rb Ukiryu::Execution::ExecutionMetadata')

# formatted_duration / JSON float over raw Float durations (started_at 0.0)
rng = Random.new(20_261_112)
floats = [0.0, -0.0, 1e-9, 1e-7, 4.9999e-6, 0.000005, 0.0049999, 0.005, 0.0125, 0.5, 0.99999, 0.999995, 1.0,
          1.0005, 1.2345, 9.9995, 59.9994, 59.9995, 59.99999, 60.0, 60.04, 60.05, 61.25, 119.95, 3599.96, 1e6,
          1e15, 1e16, 2.5e-10, -0.5, -1.5, -61.0, 0.1 + 0.2, 1.0 / 3]
300.times { floats << rng.rand * 0.002 }
300.times { floats << rng.rand * 2 }
300.times { floats << rng.rand * 120 }
300.times { floats << rng.rand * 100_000 }
300.times { floats << (rng.rand(2_000_000) / 1000.0) }
fd_rows = floats.map do |d|
  m = Ukiryu::Execution::ExecutionMetadata.new(started_at: 0.0, finished_at: d, timeout: nil)
  dur = m.duration
  { 'bits' => [dur].pack('G').unpack1('H*'), 'formatted' => m.formatted_duration,
    'to_s' => dur.to_s, 'json' => JSON.generate(dur),
    'ms_json' => JSON.generate(m.duration_milliseconds) }
end
write('duration_format', fd_rows, 'lib/ukiryu/execution/metadata.rb #formatted_duration; json gem generate(Float)')

# ---------------------------------------------------------------- CommandInfo
EXES = ['/usr/bin/convert', 'convert', '/a/b/', '', '/', 'rel/dir/tool', '//x//', 'C:\\x\\y.exe', 'name with space', 'ü/ñ"q'].freeze
ARGS = [[], ['-v'], ['in put.png', 'out.png'], ['--opt=1', '', "a\nb"]].freeze
ci_rows = []
EXES.each_with_index do |exe, i|
  args = ARGS[i % ARGS.size]
  shell = [nil, :bash, :zsh, :powershell][i % 4]
  tool = [nil, 'jq', :imagemagick][i % 3]
  cmd = [nil, 'process', :convert][i % 3]
  full = ([exe] + args).join(' ')
  c = Ukiryu::Execution::CommandInfo.new(executable: exe, arguments: args, full_command: full, shell: shell,
                                         tool_name: tool, command_name: cmd)
  ci_rows << { 'input' => { 'executable' => exe, 'arguments' => args, 'full_command' => full, 'shell' => shell&.to_s,
                            'tool_name' => tool&.to_s, 'command_name' => cmd&.to_s },
               'expect' => { 'executable_name' => c.executable_name, 'argument_count' => c.argument_count,
                             'to_s' => c.to_s, 'inspect' => c.inspect, 'to_h_json' => JSON.generate(c.to_h) } }
end
write('command_info', ci_rows, "lib/ukiryu/execution/command_info.rb Ukiryu::Execution::CommandInfo (File.basename on #{RUBY_PLATFORM})")

# ---------------------------------------------------------------- Result
res_rows = []
12.times do |i|
  ci = ci_rows[i % ci_rows.size]['input']
  out = output_rows[1 + (i % (output_rows.size - 1))]['input']
  md = meta_rows[(i * 11) % meta_rows.size]['input']
  c = Ukiryu::Execution::CommandInfo.new(executable: ci['executable'], arguments: ci['arguments'],
                                         full_command: ci['full_command'], shell: ci['shell']&.to_sym,
                                         tool_name: ci['tool_name'], command_name: ci['command_name'])
  o = Ukiryu::Execution::Output.new(stdout: out['stdout'], stderr: out['stderr'], exit_status: out['exit_status'])
  rt = ->(t) { t && Time.at(t['seconds'], t['nanos'], :nsec, in: t['utc'] ? 'UTC' : t['utc_offset']) }
  to = md['timeout'] && (md['timeout']['int'] || md['timeout']['float'].to_f)
  m = Ukiryu::Execution::ExecutionMetadata.new(started_at: rt.call(md['started_at']), finished_at: rt.call(md['finished_at']), timeout: to)
  r = Ukiryu::Execution::Result.new(command_info: c, output: o, metadata: m)
  res_rows << { 'input' => { 'command_info' => ci, 'output' => out, 'metadata' => md },
                'expect' => { 'command' => r.command, 'executable' => r.executable, 'executable_name' => r.executable_name,
                              'stdout' => r.stdout, 'stderr' => r.stderr, 'status' => r.status, 'exit_code' => r.exit_code,
                              'exit_status' => r.exit_status, 'duration' => r.duration&.to_s, 'execution_time' => r.execution_time&.to_s,
                              'success' => r.success?, 'failure' => r.failure?, 'output' => r.output, 'error_output' => r.error_output,
                              'stdout_lines' => r.stdout_lines, 'stderr_lines' => r.stderr_lines,
                              'to_json' => r.to_json, 'to_s' => r.to_s, 'inspect' => r.inspect } }
end
write('result', res_rows, 'lib/ukiryu/execution/result.rb Ukiryu::Execution::Result')

# ---------------------------------------------------------------- Errors
classes = Ukiryu::Errors.constants.map { |c| Ukiryu::Errors.const_get(c) }.select { |c| c.is_a?(Class) }.sort_by(&:name)
err_rows = classes.map do |k|
  e = k.new
  e2 = k.new('custom message')
  row = { 'class' => k.name, 'superclass' => k.superclass.name,
          'ancestors' => k.ancestors.take_while { |a| a != Object }.map(&:to_s),
          'default_message' => e.message, 'message' => e2.message, 'suggestions' => e.suggestions,
          'is_a' => classes.select { |c| e.is_a?(c) }.map(&:name), 'ruby_platform' => RUBY_PLATFORM }
  if k == Ukiryu::Errors::TimeoutError
    row['timeout_kwarg'] = [nil, 30, 0.5].map do |t|
      x = k.new('m', timeout: t)
      { 'timeout' => num_json(t), 'got' => num_json(x.timeout), 'message' => x.message }
    end
    row['timeout_default'] = num_json(e.timeout)
  end
  row
end
write('errors', err_rows, 'lib/ukiryu/errors.rb Ukiryu::Errors::*')

# ---------------------------------------------------------------- Executor messages
fe_rows = []
[['/usr/bin/false', "'/usr/bin/false'", { stdout: '', stderr: '', status: 1 }],
 ['tool', "tool 'a b'", { stdout: "  out\n", stderr: "err line\n\n", status: 42 }],
 ['x', 'x', { stdout: nil, stderr: nil, status: 137 }],
 ['multi', 'multi -v', { stdout: "l1\nl2\n", stderr: "\0e\0", status: -1 }],
 ['u', 'u', { stdout: 'ü', stderr: '日本', status: 255 }]].each do |exe, cmd, res|
  fe_rows << { 'input' => { 'executable' => exe, 'command' => cmd, 'stdout' => res[:stdout], 'stderr' => res[:stderr], 'status' => res[:status] },
               'expect' => { 'message' => Ukiryu::Executor.send(:format_error, exe, cmd, res) } }
end
write('format_error', fe_rows, 'lib/ukiryu/executor.rb Executor.format_error (private, called via send)')

# ---------------------------------------------------------------- Exit-code mapping (Response::Base)
FakeProfile = Struct.new(:cmds, :exit_codes) do
  def command(name) = cmds[name]
end
FakeCmd = Struct.new(:exit_codes)
FakeTool = Struct.new(:profile)
FakeToolProfile = Struct.new(:compatible_profile)
TABLES = {
  'none' => nil,
  'empty' => Ukiryu::Models::ExitCodes.new,
  'standard' => Ukiryu::Models::ExitCodes.new(standard: { '0' => 'success', '1' => 'general_error' }),
  'custom' => Ukiryu::Models::ExitCodes.new(custom: { '3' => 'merge_conflict', '1' => 'custom_error', '5' => 'success' }),
  'both' => Ukiryu::Models::ExitCodes.new(standard: { '0' => 'success', '1' => 'general_error', '2' => 'usage' },
                                          custom: { '1' => 'override', '4' => 'permission_denied' })
}.freeze
TABLE_SPEC = TABLES.transform_values { |t| t && { 'standard' => t.standard, 'custom' => t.custom } }
map_rows = []
mk_result = lambda do |status, tool, cmd|
  Ukiryu::Execution::Result.new(
    command_info: Ukiryu::Execution::CommandInfo.new(executable: 'x', arguments: [], full_command: 'x', tool_name: tool, command_name: cmd),
    output: Ukiryu::Execution::Output.new(stdout: '', stderr: '', exit_status: status),
    metadata: Ukiryu::Execution::ExecutionMetadata.new(started_at: Time.at(0), finished_at: Time.at(1, 500_000_000, :nsec), timeout: nil)
  )
end
TABLES.each_key do |cmd_t|
  [true, false].each do |cmd_present|
    TABLES.each_key do |prof_t|
      [0, 1, 2, 3, 4, 5, 99, -1].each do |status|
        profile = FakeProfile.new(cmd_present ? { 'run' => FakeCmd.new(TABLES[cmd_t]) } : {}, TABLES[prof_t])
        Ukiryu::Tool.define_singleton_method(:find_by) { |_n| FakeTool.new(FakeToolProfile.new(profile)) }
        resp = Ukiryu::Response::Base.new(mk_result.call(status, 'tool', 'run'))
        map_rows << { 'input' => { 'command_present' => cmd_present, 'command_table' => cmd_t, 'profile_table' => prof_t, 'status' => status },
                      'expect' => { 'meaning' => resp.exit_code_meaning, 'format_meaning' => resp.format_meaning,
                                    'success' => resp.success?, 'to_s' => resp.to_s } }
      end
    end
  end
end
# lookup short-circuits that return nil before any table is read
Ukiryu::Tool.define_singleton_method(:find_by) { |_n| nil }
[[nil, 'run'], ['tool', nil], ['tool', 'run']].each do |tool, cmd|
  resp = Ukiryu::Response::Base.new(mk_result.call(1, tool, cmd))
  map_rows << { 'input' => { 'lookup' => 'tool_missing', 'tool_name' => tool, 'command_name' => cmd, 'status' => 1 },
                'expect' => { 'meaning' => resp.exit_code_meaning } }
end
map_rows.unshift({ 'tables' => TABLE_SPEC })
begin
  Ukiryu::Response::Base.new(mk_result.call(0, nil, nil)).timed_out?
  map_rows << { 'timed_out' => 'returned' }
rescue NoMethodError => e
  map_rows << { 'timed_out' => { 'class' => e.class.name, 'message' => e.message } }
end
write('exit_code_mapping', map_rows, 'lib/ukiryu/response/base.rb #exit_code_meaning/#format_meaning with Tool.find_by stubbed; lib/ukiryu/models/exit_codes.rb')

# ---------------------------------------------------------------- JSON failures
je_rows = [Float::INFINITY, -Float::INFINITY, Float::NAN].map do |to|
  m = Ukiryu::Execution::ExecutionMetadata.new(started_at: Time.at(0, in: 'UTC'), finished_at: Time.at(2, in: 'UTC'), timeout: to)
  r = Ukiryu::Execution::Result.new(
    command_info: Ukiryu::Execution::CommandInfo.new(executable: 'x', arguments: [], full_command: 'x'),
    output: Ukiryu::Execution::Output.new(stdout: '', stderr: '', exit_status: 0), metadata: m
  )
  begin
    r.to_json
    { 'timeout' => to.to_s, 'expect' => 'generated' }
  rescue StandardError => e
    { 'timeout' => to.to_s, 'timed_out' => m.timed_out?, 'expect' => { 'class' => e.class.name, 'message' => e.message } }
  end
end
write('json_errors', je_rows, "lib/ukiryu/execution/result.rb #to_json (json #{JSON::VERSION})")

File.write(File.join(OUT, 'PROVENANCE.json'), JSON.pretty_generate(
  'oracle' => 'ukiryu/ukiryu', 'sha' => SHA, 'ruby' => RUBY_DESCRIPTION, 'json_gem' => JSON::VERSION,
  'platform' => RUBY_PLATFORM, 'script' => 'handoffs/batch-03/K20-RESULTS/capture.rb'
) + "\n")
