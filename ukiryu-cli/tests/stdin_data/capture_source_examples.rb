# frozen_string_literal: true
# Run from the pinned oracle with its bundle; compare semantic projections
# asserted by the 15 original cli_stdin_spec examples, not volatile timings.
require 'json'
require 'open3'
require 'tmpdir'
require 'shellwords'
require 'rbconfig'
require 'digest'

output = File.expand_path(ARGV.fetch(0))
reference = File.realpath(ARGV.fetch(1, Dir.pwd))
Dir.chdir(reference)
pin = 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
abort 'wrong oracle' unless `git rev-parse HEAD`.strip == pin && `git status --porcelain`.empty?
root = Dir.pwd
ruby_cli = [RbConfig.ruby, '-I', File.join(root, 'lib'), File.join(root, 'exe/ukiryu')]
env = { 'UKIRYU_REGISTER' => File.join(root, 'spec/fixtures/register'),
        'UKIRYU_SCHEMA_PATH' => File.join(root, 'spec/fixtures/tool.schema.yaml'),
        'SHELL' => '/bin/bash', 'NO_COLOR' => '1' }
cases = [
  ['flag_pipe', ['--stdin', 'filter=".test"', '--format=json'], '{"test":"value"}', 'json'],
  ['multiline', ['--stdin', 'filter=".lines"', '--format=json'], "{\n\"lines\":[\"line1\",\"line2\",\"line3\"]\n}", 'json'],
  ['dash_pipe', ['stdin=-', 'filter=".foo"', '--format=json'], '{"foo":"bar"}', 'json'],
  ['flag_equivalence', ['stdin=-', 'filter=".test"', '--format=json'], '{"test":"value"}', 'json'],
  ['file', ['stdin=@{dir}/input.json', 'filter=".file"', '--format=json'], nil, 'json'],
  ['missing_file', ['stdin=@{dir}/absent.json', 'filter="."'], nil, 'missing'],
  ['direct_data', ['stdin=@{dir}/direct.json', 'filter=.test', '--format=json'], nil, 'json'],
  ['preview', ['--stdin', '-D', 'filter="."'], '{"test":"value"}', 'preview'],
  ['long_preview', ['--stdin', '-D', 'filter="."'], 'x' * 200, 'long_preview'],
  ['json', ['--stdin', 'filter=".test"', '--format=json'], '{"test":"value"}', 'json'],
  ['yaml', ['--stdin', 'filter=".test"', '--format=yaml'], '{"test":"value"}', 'yaml'],
  ['pipeline', ['--stdin', 'filter=.items', '--format=json'], '{"items":[1,2,3]}', 'pipeline'],
  ['curl_output', ['--stdin', 'filter=.slideshow.title', '--format=json'], '{"slideshow":{"title":"Test Presentation"}}', 'json'],
  ['raw', ['--stdin', '--raw', 'filter=".test"'], '{"test":"value"}', 'raw'],
  ['raw_error', ['--stdin', '--raw', 'filter=".foo[bar"'], '{}', 'error']
]
rows = Dir.mktmpdir('ukiryu-cli-stdin') do |dir|
  File.write(File.join(dir, 'input.json'), '{"file":"content"}')
  File.write(File.join(dir, 'direct.json'), '{"test":"value"}')
  cases.map do |id, args, input, kind|
    argv = [*ruby_cli, 'exec', 'jq', 'process', *args.map { |a| a.gsub('{dir}', dir) }]
    if kind == 'pipeline'
      command = "printf %s #{Shellwords.escape(input)} | #{Shellwords.join(argv)} | head -10"
      stdout, stderr, status = Open3.capture3(env, 'bash', '-c', command)
    else
      stdout, stderr, status = Open3.capture3(env, *argv, stdin_data: input.to_s)
    end
    projection = case kind
                 when 'json'
                   parsed = JSON.parse(stdout)
                   abort "#{id}: #{stdout} #{stderr}" unless parsed.key?('output')
                   { 'command_stdout' => parsed.fetch('output').fetch('stdout') }
                 when 'raw' then { 'stdout' => stdout, 'stderr' => stderr }
                 when 'missing' then { 'file_not_found' => (stdout + stderr).include?('File not found') }
                 when 'preview' then %w[DRY\ RUN stdin: test value].to_h { |s| [s, (stdout + stderr).include?(s)] }
                 when 'long_preview' then { 'ellipsis' => stdout.include?('...'), 'full_input' => stdout.include?(input) }
                 when 'yaml' then { 'stdout_label' => stdout.include?('stdout:'), 'value' => stdout.include?('value') }
                 when 'pipeline' then { 'array' => stdout.include?('[') }
                 when 'error' then { 'stderr_nonempty' => !stderr.empty?, 'error' => (stdout + stderr).include?('error'),
                                    'syntax_error' => (stdout + stderr).include?('syntax error') }
                 end
    { 'id' => id, 'args' => args, 'stdin' => input, 'kind' => kind,
      'expected' => projection.merge('exit_status' => status.exitstatus) }
  end
end
out = { 'oracle' => pin, 'ruby' => RUBY_VERSION, 'engine' => RUBY_ENGINE, 'host' => RUBY_PLATFORM,
        'generator_commit' => Open3.capture2('git', '-C', File.dirname(__FILE__), 'rev-parse', 'HEAD').first.strip,
        'generator_sha256' => Digest::SHA256.file(__FILE__).hexdigest,
        'jq' => Open3.capture2('jq', '--version').first.strip,
        'spec_sha256' => Digest::SHA256.file('spec/ukiryu/cli_stdin_spec.rb').hexdigest,
        'file_content' => '{"file":"content"}', 'direct_content' => '{"test":"value"}', 'rows' => rows }
File.write(output, JSON.pretty_generate(out) + "\n")
