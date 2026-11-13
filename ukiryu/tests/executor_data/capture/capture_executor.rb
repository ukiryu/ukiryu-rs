# frozen_string_literal: true

# Frozen observations of the pinned Ruby executor (lib/ukiryu/executor.rb,
# lib/ukiryu/shell/{unix_base,tcsh,powershell}.rb) on a Linux host.
# Usage: ruby capture_executor.rb OUT.json
require 'json'
require 'tmpdir'
require 'tempfile'
require 'ukiryu'

E = Ukiryu::Executor

def bin(s)
  { 'hex' => s.b.unpack1('H*'), 'encoding' => s.encoding.to_s, 'valid' => s.valid_encoding? }
end

def run_case(id)
  t0 = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  row = { 'id' => id }
  begin
    r = yield
    row['outcome'] = 'result'
    row['class'] = r.class.name
    row['stdout'] = r.stdout.valid_encoding? ? r.stdout : nil
    row['stdout_bytes'] = bin(r.stdout)
    row['stderr'] = r.stderr
    row['exit_status'] = r.exit_status
    row['success'] = r.success?
    row['executable'] = r.command_info.executable
    row['arguments'] = r.command_info.arguments
    row['full_command'] = r.command_info.full_command
    row['shell'] = r.command_info.shell.inspect
    row['timeout'] = r.metadata.timeout
    row['started_at_class'] = r.started_at.class.name
  rescue Exception => e # rubocop:disable Lint/RescueException
    row['outcome'] = 'raise'
    row['class'] = e.class.name
    row['message'] = e.message
    row['timeout_attr'] = e.respond_to?(:timeout) ? e.timeout : :no_method
    row['responds_to_result'] = e.respond_to?(:result)
  end
  el = Process.clock_gettime(Process::CLOCK_MONOTONIC) - t0
  row['elapsed_lt_2s'] = el < 2.0
  row['elapsed_evidence_only'] = el.round(1)
  row
end

rows = []

# Required options.
rows << run_case('missing_shell') { E.execute('echo', ['x'], timeout: 30) }
rows << run_case('missing_timeout') { E.execute('echo', ['x'], shell: :bash) }
rows << run_case('missing_env_defaults') { E.execute('echo', ['x'], shell: :bash, timeout: 30) }
rows << run_case('timeout_zero_runs_unbounded') { E.execute('sh', ['-c', 'sleep 0.2; echo done'], shell: :bash, timeout: 0) }
rows << run_case('timeout_negative') { E.execute('echo', ['x'], shell: :bash, timeout: -1) }

# Success, capture and command info.
rows << run_case('echo_test') { E.execute('echo', ['test'], shell: :bash, timeout: 30) }
rows << run_case('echo_hello_world') { E.execute('echo', ['hello world'], shell: :bash, timeout: 30) }
rows << run_case('special_chars') { E.execute('echo', ["hello \"quoted\" test's"], shell: :bash, timeout: 90, allow_failure: true) }
rows << run_case('timeout_float_success') { E.execute('sleep', ['0.01'], shell: :bash, timeout: 5, allow_failure: true) }

# Exit codes.
rows << run_case('exit_42_allow_failure') { E.execute('sh', ['-c', 'exit 42'], shell: :bash, timeout: 30, allow_failure: true) }
rows << run_case('exit_1_raises') { E.execute('sh', ['-c', 'exit 1'], shell: :bash, timeout: 30) }
rows << run_case('stderr_in_error') { E.execute('sh', ['-c', 'echo error >&2; exit 1'], shell: :bash, timeout: 30) }
rows << run_case('false_raises') { E.execute('false', [], shell: :bash, timeout: 30) }
rows << run_case('exit_42_raises') { E.execute('sh', ['-c', 'exit 42'], shell: :bash, timeout: 90) }
rows << run_case('stdout_and_stderr_stripped') { E.execute('sh', ['-c', 'printf "  out \n"; printf "\n err  \n" >&2; exit 3'], shell: :bash, timeout: 30) }
rows << run_case('signal_term') { E.execute('sh', ['-c', 'kill -TERM $$'], shell: :bash, timeout: 30, allow_failure: true) }
rows << run_case('missing_executable') { E.execute('ukiryu_no_such_exe_xyz', [], shell: :bash, timeout: 30, allow_failure: true) }

# Timeout (the source 10-second case) and whether the child survives.
marker = '10.0371'
rows << run_case('timeout_sleep_10') { E.execute('sleep', [marker], shell: :bash, timeout: 0.01) }.tap { |r|
  sleep 0.3
  r['child_alive_after_timeout'] = system("pgrep -fx 'sleep #{marker}' > /dev/null")
  system("pkill -f 'sleep #{marker}'")
}
rows << run_case('timeout_int_message') { E.execute('sleep', ['10.0372'], shell: :bash, timeout: 1) }.tap { system("pkill -f 'sleep 10.0372'") }

# Environment (D1 frozen observations).
rows << run_case('env_hash_var') { E.execute('sh', ['-c', 'echo "$TEST_VAR"'], env: { 'TEST_VAR' => 'test_value' }, shell: :bash, timeout: 30, allow_failure: true) }
rows << run_case('env_hash_spaces') { E.execute('sh', ['-c', 'echo "$TEST_VAR"'], env: { 'TEST_VAR' => 'value with spaces' }, shell: :bash, timeout: 30, allow_failure: true) }
rows << run_case('env_empty_hash_inherits_home') { E.execute('sh', ['-c', 'echo "${HOME:+home-set}"'], env: {}, shell: :bash, timeout: 30, allow_failure: true) }
rows << run_case('env_object_without_home_still_inherits') { E.execute('sh', ['-c', 'echo "${HOME:+home-set}|$ONLY"'], env: Ukiryu::Environment.new({ 'ONLY' => '1' }), shell: :bash, timeout: 30, allow_failure: true) }
ENV['DISPLAY'] = ':99'
rows << run_case('display_parent_set') { E.execute('sh', ['-c', 'echo "${DISPLAY-unset}"'], env: {}, shell: :bash, timeout: 30) }
rows << run_case('display_user_set') { E.execute('sh', ['-c', 'echo "${DISPLAY-unset}"'], env: { 'DISPLAY' => ':5' }, shell: :bash, timeout: 30) }
rows << run_case('display_empty_explicit_parent_set') { E.execute('sh', ['-c', 'echo "${DISPLAY-unset}"'], env: { 'DISPLAY' => '' }, shell: :bash, timeout: 30) }
rows << run_case('display_env_object_parent_set') { E.execute('sh', ['-c', 'echo "${DISPLAY-unset}"'], env: Ukiryu::Environment.new({ 'DISPLAY' => ':7' }), shell: :bash, timeout: 30) }
ENV.delete('DISPLAY')
rows << run_case('display_absent_parent_unset') { E.execute('sh', ['-c', 'echo "${DISPLAY-unset}"'], env: {}, shell: :bash, timeout: 30) }
rows << run_case('display_user_set_parent_unset') { E.execute('sh', ['-c', 'echo "${DISPLAY-unset}"'], env: { 'DISPLAY' => ':5' }, shell: :bash, timeout: 30) }

# Local time zone of the result timestamps (Time.now).
def time_row(id, tz)
  saved = ENV['TZ']
  ENV['TZ'] = tz
  r = E.execute('echo', ['x'], shell: :bash, timeout: 30)
  s = r.started_at
  meta = r.to_h[:metadata]
  { 'id' => id, 'tz' => tz, 'utc_offset' => s.utc_offset, 'utc' => s.utc?,
    'to_s_zone' => s.to_s[-5..], 'to_h_started_at_zone' => meta[:started_at].to_s[-5..],
    'finished_utc_offset' => r.finished_at.utc_offset, 'nsec_in_range' => s.nsec.between?(0, 999_999_999),
    'to_s_shape' => s.to_s.gsub(/\d/, 'D') }
ensure
  saved ? ENV['TZ'] = saved : ENV.delete('TZ')
end
time_rows = [time_row('tz_karachi', 'Asia/Karachi'), time_row('tz_utc', 'UTC'), time_row('tz_st_johns', 'America/St_Johns')]

# Working directory.
Dir.mktmpdir do |dir|
  File.write(File.join(dir, 'test.txt'), 'test content')
  rows << run_case('cwd_cat') { E.execute('cat', ['test.txt'], cwd: dir, shell: :bash, timeout: 30, allow_failure: true) }
end

# stdin.
rows << run_case('stdin_string') { E.execute('/bin/cat', [], stdin: 'test data', shell: :bash, timeout: 90) }
rows << run_case('stdin_multiline') { E.execute('/bin/cat', [], stdin: "line1\nline2\nline3", shell: :bash, timeout: 90) }
rows << run_case('stdin_binary') { E.execute('/bin/cat', [], stdin: "\x00\x01\x02\x03", shell: :bash, timeout: 90) }
rows << run_case('stdin_empty_string') { E.execute('/bin/cat', [], stdin: '', shell: :bash, timeout: 30) }
rows << run_case('stdin_absent_is_closed') { E.execute('/bin/cat', [], shell: :bash, timeout: 5) }
rows << run_case('stdin_head_epipe') { E.execute('/usr/bin/head', ['-n', '1'], stdin: "line1\nline2\nline3\n", shell: :bash, timeout: 90) }
rows << run_case('stdin_head_epipe_large') { E.execute('/usr/bin/head', ['-n', '1'], stdin: "line\n" * 200_000, shell: :bash, timeout: 30) }
rows << run_case('stdin_io_file') do
  Tempfile.create('stdin_test') do |f|
    f.write('file content')
    f.rewind
    E.execute('/bin/cat', [], stdin: f, shell: :bash, timeout: 90)
  end
end
big = 'x' * 1_000_000
rows << run_case('stdin_large_cat_deadlock') { E.execute('/bin/cat', [], stdin: big, shell: :bash, timeout: 3) }.tap { |r| r['input_bytes'] = big.bytesize }

# Output encodings and large streams.
rows << run_case('stdout_utf8') { E.execute('printf', ['h\303\251'], shell: :bash, timeout: 30) }
rows << run_case('stdout_invalid_utf8') { E.execute('printf', ['a\377b'], shell: :bash, timeout: 30) }
rows << run_case('large_both_streams') do
  E.execute('sh', ['-c', 'head -c 1000000 /dev/zero | tr "\\0" o; head -c 1000000 /dev/zero | tr "\\0" e >&2'], shell: :bash, timeout: 30)
end.tap { |r| r['stdout_len'] = r.delete('stdout')&.bytesize; r['stderr_len'] = r.delete('stderr')&.bytesize; r.delete('stdout_bytes') }

# Inherited descendant holding the pipes: Open3 waits until the pipe closes.
t0 = Process.clock_gettime(Process::CLOCK_MONOTONIC)
row = run_case('descendant_holds_pipe') { E.execute('sh', ['-c', 'sleep 1.5 & echo done'], shell: :bash, timeout: 30) }
row['elapsed_ge_1_4s'] = (Process.clock_gettime(Process::CLOCK_MONOTONIC) - t0) >= 1.4
rows << row
row = run_case('descendant_detached_pipes') { E.execute('sh', ['-c', 'sleep 1.5 >/dev/null 2>&1 & echo done'], shell: :bash, timeout: 30) }
rows << row

# Shell not on PATH (fish), via the cached search paths.
saved = Ukiryu::Platform.executable_search_paths.dup
Ukiryu::Platform.executable_search_paths.replace([Dir.mktmpdir])
rows << run_case('shell_not_found') { E.execute('echo', ['x'], shell: :zsh, timeout: 30) }
rows << run_case('tcsh_not_found') { E.execute('echo', ['x'], shell: :tcsh, timeout: 30) }
Ukiryu::Platform.executable_search_paths.replace(saved)

# extract_status table (the spec doubles), on Struct stand-ins.
St = Struct.new(:exited?, :exitstatus, :signaled?, :termsig, :stopped?, :stopsig)
status_rows = {
  'exited_42' => St.new(true, 42, false, nil, false, nil),
  'signaled_15' => St.new(false, nil, true, 15, false, nil),
  'stopped_19' => St.new(false, nil, false, nil, true, 19),
  'unknown' => St.new(false, nil, false, nil, false, nil)
}.map { |k, s| { 'id' => k, 'status' => E.extract_status(s) } }

out = {
  'oracle' => {
    'repo' => 'ukiryu/ukiryu',
    'sha' => `git -C /home/apple/src/ukiryu/ukiryu rev-parse HEAD`.strip,
    'files' => %w[lib/ukiryu/executor.rb lib/ukiryu/shell/unix_base.rb lib/ukiryu/shell/tcsh.rb lib/ukiryu/errors.rb],
    'ruby' => RUBY_DESCRIPTION,
    'host' => RUBY_PLATFORM,
    'bash' => `bash --version`.lines.first.strip
  },
  'rows' => rows,
  'extract_status' => status_rows,
  'time_zone' => time_rows
}
File.write(ARGV.fetch(0), JSON.pretty_generate(out) + "\n")
