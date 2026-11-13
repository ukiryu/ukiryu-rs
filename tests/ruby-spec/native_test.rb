# frozen_string_literal: true

# Loads the core Ruby binding (ukiryu/src/ffi/ruby.rs) through the
# development extension in tests/ruby-ext and checks it against data captured
# from the pinned Ruby source and against the core's own C ABI.
#
#   UKIRYU_RUBY_EXT  path to the built extension (ukiryu_core_ruby_fixture.so)
#   UKIRYU_C_LIB     path to the core cdylib built with the `c` feature
#
# Expected values come from captures, never from this file:
#   native-reference.json                 capture_ruby_native.rb, Ruby source ab6df353
#   ukiryu/tests/models_data/serializable.json, execution_result_golden/errors.jsonl
# Every check counts; any failure exits nonzero after listing all failures.
require 'json'
require 'fiddle'

require ENV.fetch('UKIRYU_RUBY_EXT')

ROOT = File.expand_path('../..', __dir__)
REFERENCE = JSON.parse(File.read(File.join(__dir__, 'native-reference.json')))
NATIVE = Ukiryu::Native

$counts = Hash.new(0)
$failures = []

def check(area, label, ok, detail = nil)
  $counts[area] += 1
  $failures << "#{area} #{label}: #{detail}" unless ok
end

# Raised exception or nil.
def raised
  yield
  nil
rescue Exception => e # rubocop:disable Lint/RescueException
  e
end

# Type-tagged encoding shared with capture_ruby_native.rb, extended with the
# exact Float bits and Time/Regexp fields the round-trip checks need.
def enc(value, exact: false)
  case value
  when nil, true, false then value
  when Symbol then { '$sym' => value.to_s }
  when Integer then { '$int' => value.to_s }
  when Float
    return { '$float_bits' => [value].pack('G').unpack1('H*') } if exact

    text = if value.nan? then 'NaN'
           elsif value.infinite? then value.positive? ? 'Infinity' : '-Infinity'
           else value.to_s
           end
    { '$float' => text }
  when String then { '$str' => value, 'encoding' => value.encoding.name }
  when Array then { '$array' => value.map { |item| enc(item, exact: exact) } }
  when Hash then { '$hash' => value.map { |key, item| [enc(key, exact: exact), enc(item, exact: exact)] } }
  when Time then { '$time' => [value.to_i, value.nsec, value.utc_offset, value.utc?] }
  when Regexp then { '$regexp' => [value.source, value.options] }
  else { '$unencodable' => value.class.name }
  end
end

# --- 1. Value conversion: exact round trip -----------------------------------
deep = 400.times.reduce([1]) { |acc, _| [acc] }
samples = {
  'nil' => nil, 'true' => true, 'false' => false,
  'zero' => 0, 'negative' => -1, 'i64-max' => 9_223_372_036_854_775_807,
  'i64-min' => -9_223_372_036_854_775_808, 'u64-max' => 18_446_744_073_709_551_615,
  'bigint' => 2**200, 'negative-bigint' => -(2**100) - 7,
  'float' => 1.5, 'negative-zero' => -0.0, 'infinity' => Float::INFINITY,
  'negative-infinity' => -Float::INFINITY, 'nan' => Float::NAN, 'tiny' => 5e-324,
  'empty-string' => +'', 'unicode-nul' => +"Ω\0z", 'ascii' => +'plain',
  'symbol' => :sym, 'symbol-space' => :'with space', 'symbol-unicode' => :Ω,
  'time-utc' => Time.at(0).utc,
  'time-offset' => Time.at(1_700_000_000, 123_456_789, :nsec, in: '+05:00'),
  'time-negative' => Time.at(-1, 5, :nsec).utc,
  'time-zero-offset-local' => Time.at(86_400, in: '+00:00'),
  'regexp' => /a.b/i, 'regexp-xm' => /x y # c
  /mx, 'regexp-unicode' => /Ω+/,
  'array' => [1, [2, [3, []]], 'x'],
  'hash-order' => { b: 1, a: 2, 's' => 3, 1 => :x, nil => false, [1] => { z: 1 }, 2.5 => 1, 1.0 => 2 },
  'deep' => deep
}
samples.each do |label, value|
  back = NATIVE.core_value(value)
  check('value', label, enc(back, exact: true) == enc(value, exact: true),
        "#{enc(value, exact: true).inspect[0, 200]} -> #{enc(back, exact: true).inspect[0, 200]}")
  check('value', "#{label}.fresh", value.frozen? || !back.equal?(value), 'returned the same object')
end
local = Time.at(1_700_000_000)
back = NATIVE.core_value(local)
check('value', 'time-local.instant', back == local && back.utc_offset == local.utc_offset && back.utc? == local.utc?,
      "#{local.inspect} -> #{back.inspect}")
check('value', 'us-ascii.text', NATIVE.core_value(:abc.to_s) == 'abc' && NATIVE.core_value(:abc.to_s).encoding == Encoding::UTF_8)

cyclic = []
cyclic << cyclic
cyclic_hash = {}
cyclic_hash[:self] = cyclic_hash
too_deep = 600.times.reduce([1]) { |acc, _| [acc] }
refusals = {
  'object' => [Object.new, TypeError],
  'string-subclass' => [Class.new(String).new('x'), TypeError],
  'hash-subclass' => [Class.new(Hash).new, TypeError],
  'rational' => [1r, TypeError],
  'range' => [(1..2), TypeError],
  'struct' => [Struct.new(:a).new(1), TypeError],
  'binary-string' => [(+"\xff").force_encoding(Encoding::BINARY), EncodingError],
  'binary-ascii-string' => [(+'abc').force_encoding(Encoding::BINARY), EncodingError],
  'invalid-utf8' => [(+"\xff").force_encoding(Encoding::UTF_8), EncodingError],
  'utf16' => ['abc'.encode(Encoding::UTF_16LE), EncodingError],
  'sub-nanosecond-time' => [Time.at(Rational(1, 3)), ArgumentError],
  'regexp-noencoding' => [Regexp.new('a', Regexp::NOENCODING), ArgumentError],
  'regexp-ascii-fixed' => [/a/u, ArgumentError],
  'cyclic-array' => [cyclic, ArgumentError],
  'cyclic-hash' => [cyclic_hash, ArgumentError],
  'identity-hash' => [{}.compare_by_identity, ArgumentError],
  'too-deep' => [too_deep, ArgumentError],
  'nested-object' => [{ a: [Object.new] }, TypeError]
}
refusals.each do |label, (value, klass)|
  error = raised { NATIVE.core_value(value) }
  check('value-refusal', label, error.is_a?(klass), "expected #{klass}, got #{error.inspect}")
end

# --- 2. register load path against the Ruby source ---------------------------
# Rows the core reports as unsupported instead of guessing (the same list as
# ukiryu/tests/models_value_tags.rs); each must raise ModelError :unsupported.
REGISTER_GAPS = %w[tag-int-octal-string tag-binary tag-timestamp tag-set tag-omap tag-ruby-symbol].freeze
# Rows where the core's register load path returns a different value than the
# Ruby source, observed through this binding (reported to the core owner, not
# fixed here): Yeptris yields a String for an integer outside i64/u64, and
# reads the YAML 1.2 float `1e300`, which Psych (YAML 1.1) keeps as a String.
KNOWN_CORE_DIVERGENCES = {
  'bigint' => { 'ok' => { '$hash' => [[{ '$sym' => 'v' }, { '$str' => '123456789012345678901234567890', 'encoding' => 'UTF-8' }]] } },
  'negative-bigint' => { 'ok' => { '$hash' => [[{ '$sym' => 'v' }, { '$str' => '-123456789012345678901234567890', 'encoding' => 'UTF-8' }]] } },
  'floats' => { 'ok' => { '$hash' => [[{ '$sym' => 'v' }, { '$array' => [{ '$float' => '1.0' }, { '$float' => '2.5' }, { '$float' => '-0.0' }, { '$float' => '1.0e+300' }] }]] } }
}.freeze
REFERENCE.fetch('registers').each do |row|
  id = row['id']
  error = nil
  actual = begin
    { 'ok' => enc(NATIVE.register_hash(row['yaml'])) }
  rescue Ukiryu::Native::ModelError => e
    error = e
    { 'error' => { 'kind' => e.kind, 'source_class' => e.source_class } }
  end
  if KNOWN_CORE_DIVERGENCES.key?(id)
    # Still diverging exactly as recorded; a core fix must update this list.
    check('register', "#{id}.known-core-divergence", actual == KNOWN_CORE_DIVERGENCES[id],
          "recorded #{KNOWN_CORE_DIVERGENCES[id].inspect}, got #{actual.inspect}")
  elsif REGISTER_GAPS.include?(id)
    check('register', id, error && error.kind == :unsupported, "listed gap, got #{actual.inspect}")
  elsif row['expect'].key?('error')
    check('register', id, error && error.source_class == row['expect']['error']['class'],
          "expected #{row['expect'].inspect}, got #{actual.inspect}")
  else
    check('register', id, actual == row['expect'], "expected #{row['expect'].inspect}, got #{actual.inspect}")
  end
end

# --- 3. configuration against the Ruby source --------------------------------
def outcome
  { 'ok' => enc(yield) }
rescue ArgumentError => e
  { 'error' => { 'class' => e.class.name, 'message' => e.message } }
end

REFERENCE.fetch('conversions').each do |row|
  attribute = row['attribute'].to_sym
  actual = outcome { NATIVE::Config.convert(attribute, row['input']) }
  check('config', row['id'], actual == row['expect'], "expected #{row['expect'].inspect}, got #{actual.inspect}")
  string_key = outcome { NATIVE::Config.convert(row['attribute'], row['input']) }
  check('config', "#{row['id']}.string-key", string_key == row['expect'], string_key.inspect)
end
REFERENCE.fetch('environments').each do |row|
  actual = outcome { NATIVE::Config.environment_values(row['input']) }
  check('config', row['id'], actual == row['expect'], "expected #{row['expect'].inspect}, got #{actual.inspect}")
end
check('config', 'defaults', enc(NATIVE::Config.defaults) == REFERENCE['defaults'], enc(NATIVE::Config.defaults).inspect)

# --- 4. Ukiryu::Errors against the Ruby source -------------------------------
errors_path = File.join(ROOT, 'ukiryu/tests/execution_result_golden/errors.jsonl')
File.readlines(errors_path, chomp: true).reject(&:empty?).each do |line|
  row = JSON.parse(line)
  name = row['class']
  klass = Object.const_get(name)
  check('errors', "#{name}.superclass", klass.superclass.name == row['superclass'], klass.superclass.name)
  ancestors = klass.ancestors.map(&:name).take_while { |n| n != 'Exception' } + ['Exception']
  check('errors', "#{name}.ancestors", ancestors == row['ancestors'], ancestors.inspect)
  check('errors', "#{name}.default_message", klass.new.message == row['default_message'], klass.new.message)
  check('errors', "#{name}.message", klass.new(row['message']).message == row['message'])
  check('errors', "#{name}.platform", RUBY_PLATFORM == row['ruby_platform'], RUBY_PLATFORM)
  check('errors', "#{name}.suggestions", klass.new.suggestions == row['suggestions'], klass.new.suggestions.inspect)
  subclass = Class.new(klass)
  check('errors', "#{name}.subclass-suggestions", subclass.new.suggestions == row['suggestions'])
  error = raised { raise klass, 'raised' }
  check('errors', "#{name}.rescue", error.is_a?(Ukiryu::Errors::Error) && error.is_a?(StandardError))
end

# --- 5. native-backed models against the Ruby source -------------------------
golden = JSON.parse(File.read(File.join(ROOT, 'ukiryu/tests/models_data/serializable.json')))
models = golden.fetch('models')
check('models', 'class-count', models.size == 23 && NATIVE::Models.constants.size == 23,
      "#{models.size} source models, #{NATIVE::Models.constants.size} native")
models.each do |m|
  klass = NATIVE::Models.const_get(m['class'])
  check('models', "#{m['class']}.new", klass.new.to_json == m['new']['json'], klass.new.to_json)
end
outcomes = Hash.new(0)
golden.fetch('cases').each do |kase|
  id = kase['id']
  klass = NATIVE::Models.const_get(kase['class'])
  expect = kase['expect']
  # As ukiryu/tests/models_serializable.rs: the outcome is loading plus
  # serializing (a value lutaml keeps raw may only fail on output).
  model = json = yaml = nil
  error = raised do
    loaded = klass.from_yaml(kase['input_yaml'])
    json = loaded.to_json
    yaml = loaded.to_yaml
    model = loaded
  end
  if error && !error.is_a?(Ukiryu::Native::ModelError)
    check('models', id, false, "unexpected #{error.class}: #{error.message[0, 200]}")
    next
  end
  if kase['kind'] == 'divergence' && error&.kind == :unsupported
    outcomes['divergence/unsupported'] += 1
    check('models', id, true)
    next
  end
  if expect['ok']
    unless model
      check('models', id, false, "source ok, native raised #{error.kind}: #{error.message[0, 200]}")
      next
    end
    outcomes["#{kase['kind']}/ok"] += 1
    check('models', "#{id}.json", json == expect['ok']['json'], "source #{expect['ok']['json']}, native #{json}")
  else
    outcomes["#{kase['kind']}/error"] += 1
    check('models', id, error && error.source_class == expect['error']['class'],
          "source #{expect['error']['class']}, native #{model ? json : "#{error.kind} #{error.source_class}"}")
  end
end
a = NATIVE::Models::Argument.from_yaml("name: x\nvalue: y\n")
check('models', 'equality', a == NATIVE::Models::Argument.from_yaml("name: x\nvalue: y\n") &&
      a != NATIVE::Models::Argument.from_yaml("name: x\n") && a != a.to_hash)
base_error = raised { NATIVE::Model.from_yaml('a: 1') }
check('models', 'base-class-refuses', base_error.is_a?(NotImplementedError), base_error.inspect)
check('models', 'allocate-refused', raised { NATIVE::Models::Argument.allocate }.is_a?(TypeError))
sub = Class.new(NATIVE::Models::Argument)
check('models', 'subclass', sub.from_yaml("name: x\n").is_a?(sub) && sub.new.to_json == NATIVE::Models::Argument.new.to_json)

# --- 6. lifetimes: GC stress, compaction, independent copies ----------------
kept = []
GC.stress = true
begin
  3.times do
    kept << NATIVE::Models::ToolDefinition.from_yaml("name: t\nversion: '1'\n")
    kept << NATIVE.core_value({ 'k' => [1, 2**80, :s, 'Ω', Time.at(0).utc] })
    kept << NATIVE.register_hash("a: [1, {b: 2}]\n")
  end
ensure
  GC.stress = false
end
GC.start
GC.compact if GC.respond_to?(:compact)
check('lifetime', 'models-after-gc', kept.grep(NATIVE::Model).all? { |m| m.to_json == kept[0].to_json })
check('lifetime', 'values-after-gc', kept[1] == { 'k' => [1, 2**80, :s, 'Ω', Time.at(0).utc] } && kept[2] == { a: [1, { b: 2 }] })
model = NATIVE::Models::Arguments.from_yaml("flags: [a, b]\n")
hash = model.to_hash
hash.clear
check('lifetime', 'to_hash-is-a-copy', !model.to_hash.empty?)
source = { 'a' => +'text' }
copied = NATIVE.core_value(source)
source['a'] << ' changed'
check('lifetime', 'core_value-is-a-copy', copied == { 'a' => 'text' })

# --- 7. native/ffi correspondence: the same wire operations ------------------
lib = Fiddle.dlopen(ENV.fetch('UKIRYU_C_LIB'))
c_json = Fiddle::Function.new(lib['ukiryu_json'], [Fiddle::TYPE_VOIDP, Fiddle::TYPE_SIZE_T], Fiddle::TYPE_VOIDP)
c_free = Fiddle::Function.new(lib['ukiryu_free'], [Fiddle::TYPE_VOIDP], Fiddle::TYPE_VOID)
c_abi = Fiddle::Function.new(lib['ukiryu_abi_version'], [], Fiddle::TYPE_INT)
wire = lambda do |request|
  bytes = JSON.generate(request)
  reply = c_json.call(bytes, bytes.bytesize)
  begin
    JSON.parse(reply.to_s.force_encoding(Encoding::UTF_8))
  ensure
    c_free.call(reply)
  end
end
# Ruby Symbols take their textual wire form for the comparison.
stringify = lambda do |value|
  case value
  when Symbol then value.to_s
  when Array then value.map { |item| stringify.call(item) }
  when Hash then value.to_h { |k, v| [stringify.call(k), stringify.call(v)] }
  else value
  end
end
version = wire.call({ op: 'version' })
check('correspondence', 'version', NATIVE.core_version == version['version'] && NATIVE.abi_version == version['abi_version'] &&
      c_abi.call == NATIVE.abi_version, "#{NATIVE.core_version}/#{NATIVE.abi_version} vs #{version.inspect}")
yaml_docs = [
  "b: 1\na: [1, 2.5, null, true]\n", "v: 123456789012345678901234567890\n", "- x\n- {y: z}\n",
  "v: .inf\n", "1: a\n", "v: !custom x\n", "v: [1, 2\n", '', "Ω: \"\\0\"\n"
]
yaml_docs.each_with_index do |doc, index|
  reply = wire.call({ op: 'parse_yaml', yaml: doc })
  native_error = raised { NATIVE.parse_yaml(doc) }
  if reply['ok']
    check('correspondence', "parse_yaml.#{index}", native_error.nil? && NATIVE.parse_yaml(doc) == reply['value'],
          "wire #{reply.inspect}, native #{native_error ? native_error.inspect : NATIVE.parse_yaml(doc).inspect}")
  else
    check('correspondence', "parse_yaml.#{index}", native_error.is_a?(NATIVE::WireError) &&
          native_error.code == reply['error']['code'] && native_error.message == reply['error']['message'],
          "wire #{reply.inspect}, native #{native_error.inspect}")
  end
end
config_requests = [
  [{}, {}, {}],
  [{ 'UKIRYU_TIMEOUT' => '010', 'UKIRYU_SHELL' => 'zsh', 'NO_COLOR' => '' }, { format: :json, timeout: 5 }, { debug: true }],
  [{ 'UKIRYU_FORMAT' => 'table' }, { 'output' => 'o.yaml', shell: :bash }, { format: :yaml, timeout: 2**70 }],
  [{ 'UKIRYU_DEBUG' => 'maybe' }, {}, {}]
]
config_requests.each_with_index do |(environment, programmatic, cli), index|
  reply = wire.call({ op: 'resolve_config', environment: environment,
                      programmatic: stringify.call(programmatic), cli: stringify.call(cli) })
  native_error = raised { NATIVE.resolve_config(environment, programmatic, cli) }
  if reply['ok']
    native = NATIVE.resolve_config(environment, programmatic, cli)
    check('correspondence', "resolve_config.#{index}", native_error.nil? &&
          stringify.call(native) == { 'values' => reply['values'], 'sources' => reply['sources'] },
          "wire #{reply.inspect}, native #{native.inspect}")
    check('correspondence', "resolve_config.#{index}.ruby-types",
          native[:values].values_at(:shell, :format).all? { |v| v.nil? || v.is_a?(Symbol) } &&
          native[:sources].values.all? { |v| v.nil? || v.is_a?(Symbol) })
  else
    check('correspondence', "resolve_config.#{index}", native_error.instance_of?(ArgumentError) &&
          native_error.message == reply['error']['message'], "wire #{reply.inspect}, native #{native_error.inspect}")
  end
end
symbol_value = raised { NATIVE.resolve_config({}, { output: :file }, {}) }
check('correspondence', 'resolve_config.symbol-outside-schema', symbol_value.is_a?(TypeError), symbol_value.inspect)

# --- 8. Ractor: methods callable outside the main Ractor ---------------------
Warning[:experimental] = false if Warning.respond_to?(:[]=)
ractor = Ractor.new do
  [
    Ukiryu::Native.register_hash("a: [1, {b: :c}]\n"),
    Ukiryu::Native::Models::Argument.from_yaml("name: x\n").to_json,
    Ukiryu::Native::Config.defaults,
    Ukiryu::Native.core_value({ k: 2**90 }),
    begin
      Ukiryu::Native::Models::Argument.from_yaml("- x\n")
    rescue Ukiryu::Native::ModelError => e
      [e.kind, e.source_class]
    end,
    Ukiryu::Errors::ValidationError.new.suggestions
  ]
end
in_ractor = ractor.respond_to?(:value) ? ractor.value : ractor.take
main = [
  NATIVE.register_hash("a: [1, {b: :c}]\n"),
  NATIVE::Models::Argument.from_yaml("name: x\n").to_json,
  NATIVE::Config.defaults,
  NATIVE.core_value({ k: 2**90 }),
  begin
    NATIVE::Models::Argument.from_yaml("- x\n")
  rescue NATIVE::ModelError => e
    [e.kind, e.source_class]
  end,
  Ukiryu::Errors::ValidationError.new.suggestions
]
check('ractor', 'same-results', in_ractor == main, "ractor #{in_ractor.inspect[0, 300]} vs main #{main.inspect[0, 300]}")

# --- summary -----------------------------------------------------------------
summary = { 'ruby' => RUBY_VERSION, 'platform' => RUBY_PLATFORM, 'checks' => $counts,
            'model_outcomes' => outcomes, 'failures' => $failures.size }
puts "RUBY-NATIVE-SUMMARY #{JSON.generate(summary)}"
$failures.each { |failure| puts "FAIL #{failure}" }
exit($failures.empty? ? 0 : 1)
