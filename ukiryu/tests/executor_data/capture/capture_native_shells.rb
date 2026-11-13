# frozen_string_literal: true

# Exact output bytes of the pinned Ruby executor (ukiryu/ukiryu@ab6df353,
# lib/ukiryu/executor.rb and lib/ukiryu/shell/{powershell,cmd}.rb) for the
# PowerShell shell (`powershell` on Windows, `pwsh` elsewhere) and, on
# Windows, cmd. Run by .github/workflows/executor-reference.yml with Ruby
# 4.0.1 and the locked corpus/oracle-ruby bundle.
#
# Usage: ruby capture_native_shells.rb OUT.json
require 'json'
require 'rbconfig'
require 'tempfile'
require 'ukiryu'

E = Ukiryu::Executor
WINDOWS = Ukiryu::Platform.windows?

def hex(s)
  s.nil? ? nil : s.b.unpack1('H*')
end

def capture(id, shell, executable, args, stdin)
  opts = { shell: shell, timeout: 120, allow_failure: true }
  opts[:stdin] = stdin unless stdin.nil?
  row = { 'id' => id, 'shell' => shell.to_s, 'executable' => executable, 'arguments' => args,
          'stdin_hex' => hex(stdin) }
  begin
    r = E.execute(executable, args, opts)
    row.merge!('outcome' => 'result', 'stdout_hex' => hex(r.stdout), 'stderr_hex' => hex(r.stderr),
               'stdout_encoding' => r.stdout.encoding.to_s, 'exit_status' => r.exit_status,
               'full_command' => r.command_info.full_command)
  rescue StandardError => e
    row.merge!('outcome' => 'raise', 'class' => e.class.name, 'message' => e.message)
  end
  row
end

cat = WINDOWS ? 'more' : 'cat'
echo = WINDOWS ? ['cmd', ['/c', 'echo', 'hello']] : ['echo', ['hello']]
fail_cmd = WINDOWS ? ['cmd', ['/c', 'echo oops 1>&2 & exit 3']] : ['sh', ['-c', 'echo oops >&2; exit 3']]
inputs = {
  'ascii' => 'test data',
  'multiline' => "line1\nline2\n",
  'unicode' => "héllo 世界",
  'empty' => ''
}

rows = []
inputs.each { |name, data| rows << capture("powershell_stdin_#{name}", :powershell, cat, [], data) }
rows << capture('powershell_echo', :powershell, *echo, nil)
rows << capture('powershell_stderr_exit', :powershell, *fail_cmd, nil)
if WINDOWS
  inputs.each { |name, data| rows << capture("cmd_stdin_#{name}", :cmd, 'more', [], data) }
  rows << capture('cmd_echo', :cmd, 'echo', ['hello'], nil)
  rows << capture('cmd_exit_42', :cmd, 'cmd', ['/c', 'exit 42'], nil)
end

# What a child actually receives on stdin and writes on stdout, as bytes:
# a Ruby child hex-dumps its binary stdin and writes CR LF itself.
dump = ['-e', 'STDOUT.binmode; STDIN.binmode; print STDIN.read.unpack1(%q(H*)); print 13.chr, 10.chr']
child_rows = []
child_rows << capture('child_stdin_bytes', WINDOWS ? :cmd : :bash, RbConfig.ruby, dump, "a\nb\r\nc\x00")


ps = WINDOWS ? 'powershell' : 'pwsh'

# Diagnostics (not golden rows): the script the source builds and the
# command line the PowerShell process actually receives from Ruby.
require 'open3'
require 'stringio'
probes = []
tf = Tempfile.new('ukiryu_probe')
tf.write("a\nb\r\nc")
tf.close
probes << { 'tempfile_write_hex' => File.binread(tf.path).unpack1('H*') }
tf.unlink
[[echo[0], echo[1]], [fail_cmd[0], fail_cmd[1]]].each do |exe, args|
  saved_err = $stderr
  $stderr = StringIO.new
  ENV['UKIRYU_DEBUG_EXECUTABLE'] = '1'
  begin
    Ukiryu::Shell::PowerShell.new.execute_command(exe, args, Ukiryu::Environment.from_env, 60)
  rescue StandardError => e
    $stderr.puts("raised #{e.class}: #{e.message}")
  ensure
    ENV.delete('UKIRYU_DEBUG_EXECUTABLE')
    debug = $stderr.string
    $stderr = saved_err
  end
  script = debug[/ps_command:\n(.*)\z/m, 1].to_s.strip
  out, err, st = Open3.capture3(ps, '-NoLogo', '-Command', "[Environment]::CommandLine; #{script}")
  probes << { 'executable' => exe, 'arguments' => args, 'debug' => debug, 'script' => script,
              'probe_stdout_hex' => hex(out), 'probe_stdout' => out.dup.force_encoding('BINARY').encode('UTF-8', invalid: :replace, undef: :replace),
              'probe_stderr' => err.dup.force_encoding('BINARY').encode('UTF-8', invalid: :replace, undef: :replace),
              'probe_status' => st.exitstatus }
end
version = IO.popen([ps, '-NoLogo', '-NoProfile', '-Command', '$PSVersionTable.PSVersion.ToString()'], &:read).strip
out = {
  'oracle' => {
    'repo' => 'ukiryu/ukiryu',
    'sha' => 'ab6df353c6dc2d7faf7817a86aafcafa82579896',
    'gem_path' => Gem.loaded_specs['ukiryu']&.full_gem_path,
    'files' => %w[lib/ukiryu/executor.rb lib/ukiryu/shell/powershell.rb lib/ukiryu/shell/cmd.rb],
    'ruby' => RUBY_DESCRIPTION,
    'host_os' => RbConfig::CONFIG['host_os'],
    'powershell_program' => ps,
    'powershell_version' => version
  },
  'rows' => rows,
  'child_rows' => child_rows,
  'probes' => probes
}
File.binwrite(ARGV.fetch(0), JSON.pretty_generate(out) + "\n")
