# frozen_string_literal: true

# Regenerate the run-file request reference from the pinned Ruby source.
#
# Usage: ruby fixtures/request/capture-request-reference.rb /path/to/pinned-ruby-source
#
# The actual RunFileCommand#load_execution_request, #validate_request! and
# BaseCommand#stringify_keys methods are loaded from the pinned source and
# called directly. No gems are installed, no register is configured and no
# process is executed. Thor is replaced by its single used contract,
# `Thor::Error < StandardError`, so the capture needs only Ruby and Psych.
require 'json'
require 'yaml'
require 'digest'

PINNED = 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
root = File.expand_path(ARGV.fetch(0))
revision = IO.popen(['git', '-C', root, 'rev-parse', 'HEAD'], &:read).strip
raise "wrong reference revision #{revision}" unless revision == PINNED

module Thor
  class Error < StandardError; end
end

module Ukiryu
  module CliCommands; end
end
lib = File.join(root, 'lib', 'ukiryu', 'cli_commands')
load File.join(lib, 'base_command.rb')
load File.join(lib, 'response_formatter.rb')
load File.join(lib, 'run_file_command.rb')

# Typed value profile: every Ruby class is named so that symbols, strings,
# integers of any size, floats (including non-finite), booleans and nil keep
# their distinct identities. Mapping keys keep their own types and order.
def typed(value)
  case value
  when nil then { 'type' => 'nil' }
  when true, false then { 'type' => 'boolean', 'value' => value }
  when Symbol then { 'type' => 'symbol', 'value' => value.to_s }
  when String
    if value.encoding == Encoding::BINARY || !value.valid_encoding?
      { 'type' => 'bytes', 'encoding' => value.encoding.name, 'hex' => value.unpack1('H*') }
    else
      { 'type' => 'string', 'value' => value }
    end
  when Integer then { 'type' => 'integer', 'value' => value.to_s }
  when Float
    text = if value.nan? then 'NaN'
           elsif value.infinite? then value.positive? ? 'Infinity' : '-Infinity'
           else value.to_s
           end
    { 'type' => 'float', 'value' => text }
  when Array then { 'type' => 'array', 'items' => value.map { |item| typed(item) } }
  when Hash
    { 'type' => 'hash', 'entries' => value.map { |key, item| [typed(key), typed(item)] } }
  else
    { 'type' => 'unsupported', 'class' => value.class.name, 'inspect' => value.inspect }
  end
end

def failure(error, root)
  # Absolute paths are machine-specific; the pinned root is replaced so the
  # reference is portable while the message structure stays exact.
  { 'class' => error.class.name, 'message' => error.message.gsub(root, '$CASES') }
end

command = Ukiryu::CliCommands::RunFileCommand.allocate
cases_dir = File.join(__dir__, 'cases')
names = Dir.children(cases_dir).select { |name| name.end_with?('.yaml') }.sort
inputs = names.map { |name| [name.delete_suffix('.yaml'), File.join(cases_dir, name)] }
inputs << ['missing-file', File.join(cases_dir, 'does-not-exist.yaml')]
inputs << ['directory', cases_dir]

cases = inputs.map do |id, path|
  entry = { 'id' => id, 'path' => path.delete_prefix(cases_dir).sub(%r{\A/}, '') }
  begin
    request = command.send(:load_execution_request, path)
    entry['load'] = { 'ok' => typed(request) }
    begin
      entry['stringify_keys'] = { 'ok' => typed(command.send(:stringify_keys, request['arguments'])) }
    rescue StandardError => e
      entry['stringify_keys'] = { 'error' => failure(e, cases_dir) }
    end
  rescue StandardError => e
    entry['load'] = { 'error' => failure(e, cases_dir) }
  end
  entry
end

puts JSON.pretty_generate(
  'provenance' => {
    'source' => 'https://github.com/ukiryu/ukiryu',
    'revision' => PINNED,
    'methods' => ['Ukiryu::CliCommands::RunFileCommand#load_execution_request',
                  'Ukiryu::CliCommands::RunFileCommand#validate_request!',
                  'Ukiryu::CliCommands::BaseCommand#stringify_keys'],
    'ruby' => RUBY_VERSION,
    'psych' => Psych::VERSION,
    'gemfile_lock_sha256' => (ENV['BUNDLE_GEMFILE'] && Digest::SHA256.file("#{ENV['BUNDLE_GEMFILE']}.lock").hexdigest),
    'platform' => RUBY_PLATFORM
  },
  'cases' => cases
)
