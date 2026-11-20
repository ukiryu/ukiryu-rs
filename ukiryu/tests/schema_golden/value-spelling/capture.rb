# frozen_string_literal: true

# BUNDLE_GEMFILE=/clean/ukiryu-ab6df353/Gemfile bundle exec ruby capture.rb OUTPUT
require 'ukiryu'
require 'json-schema'
require 'date'
require 'digest'
require 'open3'

# Freeze the ambient timezone because Psych uses getlocal for unzoned Time.
ENV['TZ'] = 'UTC'
Encoding.default_external = Encoding::UTF_8
Encoding.default_internal = nil
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
def result
  x = yield
  x = { 'bytes' => x.bytes } if x.is_a?(String) && x.encoding == Encoding::BINARY && !x.ascii_only?
  { 'ok' => x }
rescue Exception => e
  { 'error' => { 'class' => e.class.to_s, 'message' => e.message.encode('UTF-8', invalid: :replace, undef: :replace) } }
end
inputs = File.join(__dir__, 'inputs.json')
rows = JSON.parse(File.read(inputs, encoding: 'UTF-8')).map do |row|
  classes = { 'none' => [], 'symbol' => [Symbol], 'document' => [Symbol, Date, Time] }.fetch(row.fetch('permitted'))
  row.merge('expect' => result do
    x = YAML.safe_load(row.fetch('yaml'), permitted_classes: classes)
    { 'to_s' => result { x.to_s }, 'inspect' => result { x.inspect }, 'json' => result { JSON.dump(x) } }
  end)
end
raise 'empty capture' if rows.empty?
payload = JSON.pretty_generate(rows) + "\n"
File.write(ARGV.fetch(0), payload)
File.write(ARGV.fetch(0) + '.provenance.json', JSON.pretty_generate({
  'default_external' => Encoding.default_external.to_s, 'default_internal' => Encoding.default_internal&.to_s,
  'source_commit' => source_commit.strip, 'oracle' => rev.strip, 'clean' => true, 'ruby' => RUBY_DESCRIPTION,
  'gems' => { 'psych' => Psych::VERSION, 'date' => Date::VERSION }.merge(%w[json json-schema].to_h { |n| [n, Gem.loaded_specs.fetch(n).version.to_s] }),
  'lock_sha256' => Digest::SHA256.file(File.join(root, 'Gemfile.lock')).hexdigest,
  'inputs_sha256' => Digest::SHA256.file(inputs).hexdigest,
  'generator_sha256' => Digest::SHA256.file(__FILE__).hexdigest,
  'payload_sha256' => Digest::SHA256.hexdigest(payload), 'TZ' => ENV.fetch('TZ')
}) + "\n")
warn "captured #{rows.size} cases"
