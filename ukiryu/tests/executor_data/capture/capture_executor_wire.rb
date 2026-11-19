# frozen_string_literal: true

# Observations of the pinned Ruby `Ukiryu::Executor.execute` for the
# `executor` wire op (tests/wire_executor.rs), on a Linux host with bash.
# Each case is the request the test sends; the row is what Ruby returned or
# raised. Usage: ruby capture_executor_wire.rb OUT.json
require 'json'
require 'tmpdir'
require 'ukiryu'

E = Ukiryu::Executor

def call(c, dir)
  opts = {}
  opts[:env] = c['env'] if c.key?('env')
  opts[:shell] = c['shell'].to_sym if c.key?('shell')
  opts[:timeout] = c['timeout'] if c.key?('timeout')
  opts[:cwd] = dir if c['cwd']
  opts[:stdin] = c['stdin'] if c.key?('stdin')
  opts[:tool_name] = c['tool_name'] if c.key?('tool_name')
  opts[:command_name] = c['command_name'] if c.key?('command_name')
  opts[:allow_failure] = c['allow_failure'] if c.key?('allow_failure')
  E.execute(c['executable'], c['args'] || [], opts)
end

def row(c)
  Dir.mktmpdir do |dir|
    File.write(File.join(dir, 'in.txt'), 'file content') if c['cwd']
    r = call(c, dir)
    { 'outcome' => 'result', 'stdout' => r.stdout, 'stderr' => r.stderr,
      'status' => r.exit_status, 'success' => r.success?,
      'executable' => r.command_info.executable, 'arguments' => r.command_info.arguments,
      'full_command' => r.command_info.full_command, 'shell' => r.command_info.shell.to_s,
      'tool_name' => r.command_info.tool_name, 'command_name' => r.command_info.command_name,
      'timeout' => r.metadata.timeout }
  rescue StandardError => e
    { 'outcome' => 'raise', 'class' => e.class.name, 'message' => e.message }
  end
end

CASES = [
  { 'id' => 'echo', 'executable' => 'echo', 'args' => %w[hello world], 'env' => {}, 'shell' => 'bash', 'timeout' => 30 },
  { 'id' => 'env_var', 'executable' => 'sh', 'args' => ['-c', 'echo "$WIRE_VAR"'], 'env' => { 'WIRE_VAR' => 'a b' }, 'shell' => 'bash', 'timeout' => 30 },
  { 'id' => 'stdin', 'executable' => 'cat', 'env' => {}, 'shell' => 'bash', 'timeout' => 30, 'stdin' => "line 1\nline 2" },
  { 'id' => 'cwd', 'executable' => 'cat', 'args' => ['in.txt'], 'env' => {}, 'shell' => 'bash', 'timeout' => 30, 'cwd' => true },
  { 'id' => 'names', 'executable' => 'echo', 'args' => ['x'], 'env' => {}, 'shell' => 'sh', 'timeout' => 30, 'tool_name' => 'echo', 'command_name' => 'say' },
  { 'id' => 'allow_failure', 'executable' => 'sh', 'args' => ['-c', 'echo out; echo err >&2; exit 42'], 'env' => {}, 'shell' => 'bash', 'timeout' => 30, 'allow_failure' => true },
  { 'id' => 'failure', 'executable' => 'sh', 'args' => ['-c', 'echo err >&2; exit 3'], 'env' => {}, 'shell' => 'bash', 'timeout' => 30 },
  { 'id' => 'float_timeout', 'executable' => 'echo', 'args' => ['x'], 'env' => {}, 'shell' => 'bash', 'timeout' => 2.5 },
  { 'id' => 'timed_out', 'executable' => 'sleep', 'args' => ['5'], 'env' => {}, 'shell' => 'bash', 'timeout' => 1 },
  { 'id' => 'missing_shell', 'executable' => 'echo', 'env' => {}, 'timeout' => 30 },
  { 'id' => 'missing_timeout', 'executable' => 'echo', 'env' => {}, 'shell' => 'bash' },
  { 'id' => 'missing_both', 'executable' => 'echo', 'env' => {} },
  { 'id' => 'unknown_shell', 'executable' => 'echo', 'env' => {}, 'shell' => 'nosuch', 'timeout' => 30 },
  { 'id' => 'unknown_shell_no_timeout', 'executable' => 'echo', 'env' => {}, 'shell' => 'nosuch' }
].freeze

out = {
  'oracle' => { 'repo' => 'ukiryu/ukiryu', 'sha' => `git -C #{File.dirname($LOADED_FEATURES.find { |f| f.end_with?('/lib/ukiryu.rb') })} rev-parse HEAD`.strip,
                'ruby' => RUBY_DESCRIPTION, 'capture_script' => 'ukiryu/tests/executor_data/capture/capture_executor_wire.rb' },
  'rows' => CASES.map { |c| { 'case' => c, 'ruby' => row(c) } }
}
File.write(ARGV.fetch(0), "#{JSON.pretty_generate(out)}\n")
