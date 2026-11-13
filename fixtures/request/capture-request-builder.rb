# frozen_string_literal: true

# Regenerate the request -> command builder reference from the pinned Ruby
# source: the request is loaded by RunFileCommand#load_execution_request,
# its arguments go through BaseCommand#stringify_keys, and the result is
# passed to CommandBuilder#build_args exactly as Tool#execute passes params.
#
# Usage (from a bundle on the corpus oracle Gemfile.lock):
#   bundle exec ruby fixtures/request/capture-request-builder.rb /path/to/pinned-ruby-source
require 'json'
require 'yaml'
require 'digest'

PINNED = 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
root = File.expand_path(ARGV.fetch(0))
revision = IO.popen(['git', '-C', root, 'rev-parse', 'HEAD'], &:read).strip
raise "wrong reference revision #{revision}" unless revision == PINNED

$LOAD_PATH.unshift File.join(root, 'lib')
require 'ukiryu'
require 'thor'
lib = File.join(root, 'lib', 'ukiryu', 'cli_commands')
load File.join(lib, 'base_command.rb')
load File.join(lib, 'response_formatter.rb')
load File.join(lib, 'run_file_command.rb')

class Harness
  include Ukiryu::CommandBuilder

  def initialize(shell, platform)
    @shell = shell
    @platform = platform
  end
end

COMMAND = {
  'name' => 'convert',
  'options' => [{ 'name' => 'quality', 'cli' => '-quality', 'type' => 'integer' }],
  'flags' => [{ 'name' => 'strip', 'cli' => '-strip' }],
  'arguments' => [
    { 'name' => 'inputs', 'variadic' => true, 'position' => '1' },
    { 'name' => 'output', 'position' => 'last' }
  ]
}.freeze

command = Ukiryu::CliCommands::RunFileCommand.allocate
definition = Ukiryu::Models::CommandDefinition.from_yaml(COMMAND.to_yaml)
cases_dir = File.join(__dir__, 'cases')
ids = %w[spec-valid builder-convert arguments-key-collision extra-top-level-fields]
cases = ids.map do |id|
  request = command.send(:load_execution_request, File.join(cases_dir, "#{id}.yaml"))
  params = command.send(:stringify_keys, request['arguments'])
  outcome = begin
    { 'ok' => Harness.new(:bash, :linux).build_args(definition, params) }
  rescue StandardError => e
    { 'error' => { 'class' => e.class.name, 'message' => e.message } }
  end
  { 'id' => id, 'build_args' => outcome }
end

puts JSON.pretty_generate(
  'provenance' => {
    'source' => 'https://github.com/ukiryu/ukiryu',
    'revision' => PINNED,
    'methods' => ['Ukiryu::CliCommands::RunFileCommand#load_execution_request',
                  'Ukiryu::CliCommands::BaseCommand#stringify_keys',
                  'Ukiryu::CommandBuilder#build_args'],
    'shell' => 'bash',
    'platform' => 'linux',
    'ruby' => RUBY_VERSION,
    'psych' => Psych::VERSION,
    'gemfile_lock_sha256' => (ENV['BUNDLE_GEMFILE'] && Digest::SHA256.hexdigest(File.binread("#{ENV['BUNDLE_GEMFILE']}.lock").gsub("\r\n", "\n"))),
    'ruby_platform' => RUBY_PLATFORM
  },
  'command' => COMMAND,
  'cases' => cases
)
