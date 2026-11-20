# frozen_string_literal: true

# Run alongside capture.rb with the same pinned bundle and TZ=UTC.
require 'ukiryu'
require 'json-schema'
require 'tempfile'
require 'digest'
require 'open3'

# Freeze the ambient timezone because Psych uses getlocal for unzoned Time.
ENV['TZ'] = 'UTC'
port_root = File.expand_path('../../../..', __dir__)
source_paths = %w[ukiryu/src/schema ukiryu/src/types/ruby.rs ukiryu/tests/schema_golden/value-spelling/capture.rb ukiryu/tests/schema_golden/value-spelling/capture_diagnostics.rb ukiryu/tests/schema_golden/value-spelling/inputs.json ukiryu/tests/schema_golden/value-spelling/diagnostic-inputs.json]
source_commit, source_status = Open3.capture2('git', '-C', port_root, 'log', '-1', '--format=%H', '--', *source_paths)
raise 'no port source commit' unless source_status.success?
root = Gem.loaded_specs.fetch('ukiryu').full_gem_path
rev, status = Open3.capture2('git', '-C', root, 'rev-parse', 'HEAD')
raise 'wrong oracle' unless status.success? && rev.strip == 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
dirty, status = Open3.capture2('git', '-C', root, 'status', '--porcelain')
raise 'dirty oracle' unless status.success? && dirty.empty?
raise 'wrong json-schema' unless Gem.loaded_specs.fetch('json-schema').version.to_s == '6.2.1'
def capture
  yield
  { 'ok' => true }
rescue Exception => e
  { 'error' => { 'class' => e.class.to_s, 'message' => e.message } }
end
input_path = File.join(__dir__, 'diagnostic-inputs.json')
inputs = JSON.parse(File.read(input_path))
rows = inputs.fetch('json').map do |text|
  f = Tempfile.new(['schema', '.json'])
  begin
    f.write(text); f.close
    { 'json' => text, 'expect' => capture { JSON::Validator.fully_validate(f.path, {}) }, 'parse_expect' => capture { JSON.parse(text) } }
  ensure
    f.close!
  end
end
rows += inputs.fetch('bytes').map do |bytes|
  f = Tempfile.new(['schema', '.yaml'])
  begin
    f.binmode; f.write(bytes.pack('C*')); f.close
    { 'bytes' => bytes, 'expect' => capture { Ukiryu::SchemaValidator.load_schema(f.path) } }
  ensure
    f.close!
  end
end
raise 'empty diagnostics' if rows.empty?
payload = JSON.pretty_generate(rows) + "\n"
File.write(ARGV.fetch(0), payload)
File.write(ARGV.fetch(0) + '.provenance.json', JSON.pretty_generate({
  'source_commit' => source_commit.strip, 'oracle' => rev.strip, 'clean' => true, 'ruby' => RUBY_DESCRIPTION,
  'psych' => Psych::VERSION, 'json' => JSON::VERSION, 'json-schema' => Gem.loaded_specs.fetch('json-schema').version.to_s,
  'lock_sha256' => Digest::SHA256.file(File.join(root, 'Gemfile.lock')).hexdigest,
  'inputs_sha256' => Digest::SHA256.file(input_path).hexdigest,
  'generator_sha256' => Digest::SHA256.file(__FILE__).hexdigest,
  'payload_sha256' => Digest::SHA256.hexdigest(payload)
}) + "\n")
warn "captured #{rows.size} diagnostics"
