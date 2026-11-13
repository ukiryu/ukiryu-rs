# Oracle capture for Ukiryu::Definition::DefinitionValidator (ukiryu ab6df353,
# json-schema 6.2.1). Usage: capture.rb <ukiryu/tests dir> <out.json>
# Records, per case, the public result (to_h, or the raised class/message) and
# what each stage produced: validate_structure (private, via send), how the
# schema resolved (public load_schema/find_schema, plus an observed call of the
# private download_and_load_schema, stubbed to return nil = download failed),
# and validate_against_schema (private, via send) when a schema loaded.
require 'json'
require 'yaml'
require 'date'
require 'ukiryu'
require 'ukiryu/definition/definition_validator'

ROOT = File.expand_path(ARGV.fetch(0))
OUT = ARGV.fetch(1)
V = Ukiryu::Definition::DefinitionValidator

$downloads = 0
V.singleton_class.send(:define_method, :download_and_load_schema) do
  $downloads += 1
  nil
end

def abs(rel) = rel && File.join(ROOT, rel)
def scrub(v)
  case v
  when String then v.gsub(ROOT, '{root}')
  when Array then v.map { |x| scrub(x) }
  when Hash then v.to_h { |k, x| [k.to_s, scrub(x)] }
  else v
  end
end

CASES = []
def c(id, yaml: nil, file: nil, schema: nil, env: nil, env_literal: nil)
  CASES << { id: id, yaml: yaml, file: file, schema: schema, env: env, env_literal: env_literal }
end

# --- structure (no schema, no env: Ruby would download; stubbed as failed)
c 'structure.name_ok', yaml: "name: tool\n"
c 'structure.name_whitespace', yaml: "name: a b\n"
c 'structure.name_digit', yaml: "name: 1tool\n"
c 'structure.name_upper', yaml: "name: Tool\n"
c 'structure.name_multiline_lower', yaml: "name: \"a\\nb\"\n"
c 'structure.name_multiline_digit_upper', yaml: "name: \"9\\nX\"\n"
c 'structure.name_trailing_newline', yaml: "name: \"tool\\n\"\n"
c 'structure.name_tab', yaml: "name: \"a\\tb\"\n"
c 'structure.name_integer', yaml: "name: 123\n"
c 'structure.name_float', yaml: "name: 1.5\n"
c 'structure.name_symbol', yaml: "name: :sym\n"
c 'structure.name_null', yaml: "name: ~\n"
c 'structure.name_false', yaml: "name: false\n"
c 'structure.name_true', yaml: "name: true\n"
c 'structure.name_empty', yaml: "name: ''\n"
c 'structure.name_array', yaml: "name: [a, b]\n"
c 'structure.name_hash', yaml: "name: {a: 1}\n"
c 'structure.name_date', yaml: "name: 2026-01-02\n"
c 'structure.name_unicode', yaml: "name: tööl\n"
c 'structure.symbol_key_wins', yaml: ":name: Sym\nname: other\n"
c 'structure.symbol_key_falsy', yaml: ":name: ~\nname: Other\n"
c 'structure.schema_string_ok', yaml: "ukiryu_schema: '1.0'\n"
c 'structure.schema_float_ok', yaml: "ukiryu_schema: 1.0\n"
c 'structure.schema_integer', yaml: "ukiryu_schema: 1\n"
c 'structure.schema_three_parts', yaml: "ukiryu_schema: '1.0.0'\n"
c 'structure.schema_null', yaml: "ukiryu_schema: ~\n"
c 'structure.schema_false', yaml: "ukiryu_schema: false\n"
c 'structure.schema_symbol_key', yaml: ":ukiryu_schema: '2.0'\n"
c 'structure.schema_symbol_key_null', yaml: ":ukiryu_schema: ~\n"
c 'structure.schema_multiline', yaml: "ukiryu_schema: \"x\\n1.2\"\n"
c 'structure.schema_date', yaml: "ukiryu_schema: 2026-01-01\n"
c 'structure.schema_array', yaml: "ukiryu_schema: [1]\n"
c 'structure.schema_v_prefix', yaml: "ukiryu_schema: v1.0\n"
c 'structure.both_warnings', yaml: "name: 9 Lives\nukiryu_schema: x\n"
c 'structure.scalar', yaml: "hello\n"
c 'structure.array', yaml: "- a\n"
c 'structure.empty', yaml: ''
c 'structure.integer_keys', yaml: "1: a\nname: ok\n"
c 'load.invalid_yaml', yaml: "a: [1\n"
c 'load.alias', yaml: "a: &x 1\nb: *x\n"
c 'load.time_name', yaml: "name: 2026-01-01 10:00:00\n"

# --- schema stage (given schema path: no fallback)
c 'schema.fixture_profile_ok', file: 'schema_golden/inputs/oracle/profiles/ghostscript_10.0.yaml',
                                schema: 'schema_golden/inputs/oracle/fixture-tool.schema.yaml'
c 'schema.fixture_minimal', yaml: "name: tool\n", schema: 'schema_golden/inputs/oracle/fixture-tool.schema.yaml'
c 'schema.fixture_structure_and_schema', yaml: "name: a B\nukiryu_schema: x\n",
                                         schema: 'schema_golden/inputs/oracle/fixture-tool.schema.yaml'
c 'schema.fixture_not_hash', yaml: "- a\n", schema: 'schema_golden/inputs/oracle/fixture-tool.schema.yaml'
c 'schema.draft4_ok', file: 'definition_validator_data/defs/draft4_ok.yaml',
                      schema: 'schema_golden/inputs/synthetic/draft4_basic.yaml'
c 'schema.draft4_bad', file: 'definition_validator_data/defs/draft4_bad.yaml',
                       schema: 'schema_golden/inputs/synthetic/draft4_basic.yaml'
c 'schema.draft4_symbol_keys', yaml: ":name: tool\n:size: 2\n", schema: 'schema_golden/inputs/synthetic/draft4_basic.yaml'
c 'schema.json_schema', yaml: "size: 1\n", schema: 'schema_golden/inputs/synthetic/json_content.json'
c 'schema.missing_file', yaml: "name: Tool\n", schema: 'definition_validator_data/none.yaml'
c 'schema.empty_path', yaml: "name: Tool\n", schema: ''
c 'schema.unknown_extension', yaml: "name: Tool\n", schema: 'definition_validator_data/defs/schema.txt'
c 'schema.invalid_yaml_schema', yaml: "name: Tool\n", schema: 'schema_golden/inputs/synthetic/invalid.yaml'
c 'schema.original_v1_unparseable', yaml: "name: tool\n", schema: 'schema_golden/inputs/oracle/v1/tool.schema.yaml'
c 'schema.false_schema', yaml: "name: Tool\n", schema: 'schema_golden/inputs/synthetic/false.yaml'
c 'schema.null_schema', yaml: "name: Tool\n", schema: 'schema_golden/inputs/synthetic/null.yaml'
c 'schema.draft7_raises', yaml: "name: tool\n", schema: 'schema_golden/inputs/synthetic/draft7.yaml'
c 'schema.draft3_unsupported', yaml: "name: tool\n", schema: 'schema_golden/inputs/synthetic/draft3.yaml'
c 'schema.given_beats_env', yaml: "size: 1\n", schema: 'schema_golden/inputs/synthetic/json_content.json',
                            env: 'schema_golden/inputs/synthetic/draft4_basic.yaml'
c 'schema.given_missing_no_env_fallback', yaml: "size: 1\n", schema: 'definition_validator_data/none.yaml',
                                          env: 'schema_golden/inputs/synthetic/json_content.json'

# --- config: UKIRYU_SCHEMA_PATH (find_schema)
c 'env.schema_used', yaml: "size: 1\n", env: 'schema_golden/inputs/synthetic/json_content.json'
c 'env.missing_file_remote', yaml: "size: 1\n", env: 'definition_validator_data/none.json'
c 'env.empty_remote', yaml: "size: 1\n", env_literal: ''
c 'env.invalid_schema_remote', yaml: "size: 1\n", env: 'schema_golden/inputs/synthetic/invalid.yaml'
c 'env.false_schema_remote', yaml: "size: 1\n", env: 'schema_golden/inputs/synthetic/false.yaml'

# --- validate_file
c 'file.minimal', file: 'definition_validator_data/defs/minimal.yaml'
c 'file.bad_name', file: 'definition_validator_data/defs/bad_name.yaml'
c 'file.broken_yaml', file: 'definition_validator_data/defs/broken.yaml'
c 'file.missing', file: 'definition_validator_data/defs/missing.yaml'
c 'file.directory', file: 'definition_validator_data/defs'

def stages(text, schema_path)
  definition = YAML.safe_load(text, permitted_classes: [Symbol, Date, Time])
  out = { 'structure' => V.send(:validate_structure, definition) }
  before = $downloads
  if schema_path
    schema = V.load_schema(schema_path)
    out['schema_stage'] = schema ? 'given' : 'given_not_loaded'
  else
    found = V.find_schema
    schema = found && V.load_schema(found)
    unless schema
      V.send(:find_and_load_schema)
      raise 'download not observed' unless $downloads == before + 1
    end
    out['schema_stage'] = schema ? 'env' : 'remote_fallback'
  end
  out['schema'] = schema ? V.send(:validate_against_schema, definition, schema) : nil
  out
rescue Psych::SyntaxError
  { 'structure' => nil, 'schema_stage' => nil, 'schema' => nil }
end

results = CASES.map do |cs|
  ENV.delete('UKIRYU_SCHEMA_PATH')
  ENV['UKIRYU_SCHEMA_PATH'] = abs(cs[:env]) if cs[:env]
  ENV['UKIRYU_SCHEMA_PATH'] = cs[:env_literal] if cs[:env_literal]
  schema_path = cs[:schema] && (cs[:schema].empty? ? '' : abs(cs[:schema]))
  row = { 'id' => cs[:id], 'input' => cs.compact.transform_keys(&:to_s) }
  begin
    result = if cs[:file]
               V.validate_file(abs(cs[:file]), schema_path: schema_path)
             else
               V.validate_string(cs[:yaml], schema_path: schema_path)
             end
    row['ok'] = result.to_h
    text = cs[:file] ? (File.file?(abs(cs[:file])) ? File.read(abs(cs[:file])) : nil) : cs[:yaml]
    row['stages'] = text ? stages(text, schema_path) : { 'structure' => nil, 'schema_stage' => nil, 'schema' => nil }
  rescue StandardError => e
    row['error'] = { 'class' => e.class.name, 'message' => e.message }
  end
  scrub(row)
ensure
  ENV.delete('UKIRYU_SCHEMA_PATH')
end

doc = {
  'provenance' => {
    'oracle' => 'ukiryu/ukiryu ab6df353c6dc2d7faf7817a86aafcafa82579896',
    'source' => 'lib/ukiryu/definition/definition_validator.rb',
    'ruby' => RUBY_VERSION, 'psych' => Psych::VERSION,
    'json-schema' => Gem.loaded_specs['json-schema']&.version&.to_s,
    'remote_fallback' => 'download_and_load_schema stubbed to return nil (download failed); each call observed',
    'capture' => 'ukiryu/tests/definition_validator_data/capture.rb <ukiryu/tests> <cases.json>'
  },
  'cases' => results
}
File.write(OUT, JSON.pretty_generate(doc) + "\n")
puts "#{results.size} cases, #{results.count { |r| r['error'] }} raised"
