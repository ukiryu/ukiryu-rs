# frozen_string_literal: true

# Regenerate tests/ruby-spec/native-reference.json from the pinned Ruby source:
#   ruby tests/ruby-spec/capture_reference.rb <ukiryu checkout at ab6df353> \
#        ukiryu/tests/models_data/register_tags.json
# Values carry an explicit type tag so comparisons do not depend on the Ruby
# version's #inspect format. No gems are installed and no process runs.
require 'json'
require 'yaml'
root = File.expand_path(ARGV.fetch(0))
revision = IO.popen(['git', '-C', root, 'rev-parse', 'HEAD'], &:read).strip
raise 'wrong reference revision' unless revision == 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
$LOAD_PATH.unshift(File.join(root, 'lib'))
require 'ukiryu/config'
require 'ukiryu/utils'

def enc(value)
  case value
  when nil, true, false then value
  when Symbol then { '$sym' => value.to_s }
  when Integer then { '$int' => value.to_s }
  when Float
    text = if value.nan? then 'NaN'
           elsif value.infinite? then value.positive? ? 'Infinity' : '-Infinity'
           else value.to_s
           end
    { '$float' => text }
  when String then { '$str' => value, 'encoding' => value.encoding.name }
  when Array then { '$array' => value.map { |item| enc(item) } }
  when Hash then { '$hash' => value.map { |key, item| [enc(key), enc(item)] } }
  else raise "unencodable #{value.class}"
  end
end

def outcome
  { 'ok' => enc(yield) }
rescue StandardError => e
  { 'error' => { 'class' => e.class.name, 'message' => e.message } }
end

conversions = []
{
  timeout: [nil, '', '0', '010', '08', '0x10', '0x_ff', '0o17', '0b10', '0d10',
            '1_000', '1__0', '+12', '  -12 ', '0_10', '0x1_0', '-0x10', '0_9',
            "\u00a01", '999999999999999999999999999999999999999'],
  debug: [nil, '', 'true', 'TRUE', '1', 'yes', 'YES', 'false', '0', 'No', 'off', ' true '],
  shell: [nil, '', 'bash', 'BASH', ' strange shell '],
  format: [nil, '', 'json', 'YAML'],
  output: [nil, '', ' ', 'a b/Ω'],
  register: [nil, '/tmp/register'],
  use_color: ['yes', 'nope'],
  future: [nil, '', '123']
}.each do |attribute, values|
  values.each_with_index do |value, index|
    conversions << { 'id' => "convert.#{attribute}.#{index}", 'attribute' => attribute.to_s, 'input' => value,
                     'expect' => outcome { Ukiryu::Config::TypeConverter.convert(attribute, value) } }
  end
end

keys = Ukiryu::Config::EnvSchema.all_attributes.map { |key| Ukiryu::Config::EnvSchema.env_key(key) } + ['NO_COLOR']
environments = [
  {}, { 'UKIRYU_DEBUG' => '' }, { 'UKIRYU_TIMEOUT' => '010' },
  { 'UKIRYU_TIMEOUT' => '08' }, { 'UKIRYU_DEBUG' => 'false' },
  { 'UKIRYU_USE_COLOR' => 'true', 'NO_COLOR' => '' },
  { 'UKIRYU_USE_COLOR' => 'invalid', 'NO_COLOR' => '' },
  { 'UKIRYU_UNKNOWN' => 'true', 'UKIRYU_SHELL' => 'BASH' },
  { 'UKIRYU_FORMAT' => 'json', 'UKIRYU_OUTPUT' => 'out.yaml', 'UKIRYU_REGISTER' => '/r',
    'UKIRYU_METRICS' => 'yes', 'UKIRYU_DRY_RUN' => '1', 'UKIRYU_TIMEOUT' => '99999999999999999999' }
].each_with_index.map do |values, index|
  keys.each { |key| ENV.delete(key) }
  values.each { |key, value| ENV[key] = value }
  { 'id' => "environment.#{index}", 'input' => values,
    'expect' => outcome { Ukiryu::Config::EnvProvider.load_all } }
end
keys.each { |key| ENV.delete(key) }
defaults = enc(Ukiryu::Config.new.to_h)

# The register load path (lib/ukiryu/register.rb): YAML.safe_load with
# Symbol permitted and aliases, then Utils.symbolize_keys; nil for empty/null.
register_path = lambda do |yaml|
  data = YAML.safe_load(yaml, permitted_classes: [Symbol], aliases: true)
  data ? Ukiryu::Utils.symbolize_keys(data) : nil
end
# Rows of ukiryu/tests/models_data/register_tags.json, re-captured with types.
tag_rows = JSON.parse(File.read(ARGV.fetch(1)))['rows'].map { |row| [row['id'], row['yaml']] }
extra_rows = [
  ['order', "b: 1\na: 2\nc: {z: 1, y: 2}\n"],
  ['bigint', "v: 123456789012345678901234567890\n"],
  ['negative-bigint', "v: -123456789012345678901234567890\n"],
  ['i64-edges', "v: [9223372036854775807, -9223372036854775808, 18446744073709551615]\n"],
  ['floats', "v: [1.0, 2.5, -0.0, 1e300]\n"],
  ['nil-false', "v: [null, ~, false, true]\n"],
  ['non-string-keys', "1: a\ntrue: b\n2.5: c\n"],
  ['unicode', "v: \"Ω\\0z\"\n"],
  ['nested', "v:\n  - a: 1\n    b: [x, {c: d}]\n"],
  ['anchor-alias', "a: &x [1, 2]\nb: *x\n"],
  ['empty', ''],
  ['null-doc', "~\n"],
  ['syntax-error', "v: [1, 2\n"]
]
registers = (tag_rows + extra_rows).map do |id, yaml|
  { 'id' => id, 'yaml' => yaml, 'expect' => outcome { register_path.call(yaml) } }
end

puts JSON.pretty_generate(
  'schema' => 'ukiryu-ruby-native-reference/1',
  'source' => revision,
  'ruby' => RUBY_VERSION,
  'ruby_platform' => RUBY_PLATFORM,
  'producer' => 'tests/ruby-spec/capture_reference.rb',
  'apis' => {
    'conversions' => 'lib/ukiryu/config/type_converter.rb TypeConverter.convert',
    'environments' => 'lib/ukiryu/config/env_provider.rb EnvProvider.load_all',
    'defaults' => 'lib/ukiryu/config.rb Config.new.to_h',
    'registers' => 'lib/ukiryu/register.rb YAML.safe_load(permitted_classes: [Symbol], aliases: true) + Utils.symbolize_keys'
  },
  'conversions' => conversions,
  'environments' => environments,
  'defaults' => defaults,
  'registers' => registers
)
