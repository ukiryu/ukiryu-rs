# frozen_string_literal: true
# Run from the pinned clean ukiryu oracle; OUTPUT is the sole argument.
require 'ukiryu'
require 'json-schema'
require 'json'
require 'digest'
require 'open3'
require 'bigdecimal'
revision, status = Open3.capture2('git', 'rev-parse', 'HEAD')
raise 'reference revision drift' unless status.success? && revision.strip == 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
dirty, status = Open3.capture2('git', 'status', '--porcelain')
raise 'dirty reference' unless status.success? && dirty.empty?
raise 'oracle drift' unless RUBY_VERSION == '4.0.1' && Gem.loaded_specs.fetch('json-schema').version.to_s == '6.2.1' && Gem.loaded_specs.fetch('bigdecimal').version.to_s == '4.1.3'
# Fiddle is a probe dependency only. The reference bundle deliberately remains
# untouched; load this installed Ruby's copy explicitly when Bundler hides it.
begin
  require 'fiddle/import'
rescue LoadError
  lib = File.join(Gem.default_dir, 'gems/fiddle-1.1.8/lib')
  ext = Dir.glob(File.join(Gem.default_dir, 'extensions/*/*/fiddle-1.1.8')).fetch(0)
  $LOAD_PATH.unshift(lib, ext)
  require 'fiddle/import'
end
module Engine
  extend Fiddle::Importer
  dlload Fiddle.dlopen(nil)
  extern 'int onig_set_match_stack_limit_size(unsigned int)'
  extern 'unsigned int onig_get_match_stack_limit_size()'
end

def observe
  {'ok'=>yield}
rescue Exception => e # NoMemoryError intentionally lies outside StandardError.
  {'error'=>{'class'=>e.class.name, 'message'=>e.message}}
end

rows = []
exponents = [
  '9223372036854775790', '9223372036854775799', '9223372036854775800',
  '9223372036854775807', '9223372036854775808', '9223372036854775809',
  '-9223372036854775790', '-9223372036854775799', '-9223372036854775800',
  '-9223372036854775807', '-9223372036854775808', '-9223372036854775809',
  '999999999999999999999999999', '-999999999999999999999999999',
  '9'*100, '-'+'9'*100, '0'*30+'2', '-0002'
]
exponents.each_with_index do |exponent, i|
  %w[1 0 -1 1.5 0.01 1000000000].each_with_index do |mantissa, j|
    [0, 1, -1, 0.3].each_with_index do |data, k|
      schema = {'multipleOf'=>"#{mantissa}e#{exponent}"}
      JSON::Validator.clear_cache
      expected = observe { JSON::Validator.fully_validate(schema, data) }
      rows << {'id'=>"decimal.#{i}.#{j}.#{k}", 'schema'=>schema, 'data'=>data, 'expect'=>expected}
    end
  end
end
# Distinguish the gem's numeric-zero short circuit from a String zero divisor,
# and keep nonnumeric documents (which bypass BigDecimal conversion) accepted.
[0, 0.0, '0', '0.0', '0e99999999999999999999', 'bad'].each_with_index do |factor, i|
  [0, 1, 'x', {}, nil].each_with_index do |data, j|
    schema = {'multipleOf'=>factor}
    JSON::Validator.clear_cache
    rows << {'id'=>"zero.#{i}.#{j}", 'schema'=>schema, 'data'=>data,
             'expect'=>observe { JSON::Validator.fully_validate(schema, data) }}
  end
end

# The engine's default limit is unlimited. A temporary limit makes its runtime
# failure deterministic without exhausting memory. Isolate it from other tests.
patterns = [
  ['\\A(a|b)*\\z', 'a'],
  ['\\A(?:a/)*\\z', 'a/'],
  ["\\A(?:a\n)*\\z", "a\n"],
  ['\\A(?:é)*\\z', 'é'],
  ['\\A(?<x>a\\g<x>?b)\\z', 'a']
]
old_limit = Engine.onig_get_match_stack_limit_size
begin
  Engine.onig_set_match_stack_limit_size(1000)
  patterns.each_with_index do |(pattern, unit), i|
    [1, 1024].each do |repeat|
      data = {'v'=>unit*repeat}
      schema = {'properties'=>{'v'=>{'pattern'=>pattern}}}
      JSON::Validator.clear_cache
      rows << {'id'=>"pattern.#{i}.#{repeat}", 'schema'=>schema,
               'repeat'=>{'unit'=>unit, 'count'=>repeat}, 'stack_limit'=>1000,
               'expect'=>observe { JSON::Validator.fully_validate(schema, data) }}
    end
  end
ensure
  Engine.onig_set_match_stack_limit_size(old_limit)
end
raise 'empty capture' if rows.empty?
raise 'missing exceptions' unless rows.any? { |row| row.dig('expect', 'error') }
raise 'missing acceptances' unless rows.any? { |row| row['expect'] == {'ok'=>[]} }
reference = {'revision'=>revision.strip, 'ruby'=>RUBY_VERSION,
             'json-schema'=>Gem.loaded_specs.fetch('json-schema').version.to_s,
             'bigdecimal'=>Gem.loaded_specs.fetch('bigdecimal').version.to_s,
             'addressable'=>Gem.loaded_specs.fetch('addressable').version.to_s,
             'fiddle'=>Fiddle::VERSION, 'lockfile_sha256'=>Digest::SHA256.file(ENV.fetch('BUNDLE_GEMFILE') + '.lock').hexdigest,
             'generator_sha256'=>Digest::SHA256.file(__FILE__).hexdigest}
text = "{\n  \"reference\": " + JSON.generate(reference) + ",\n  \"cases\": [\n" + rows.map { |row| '    ' + JSON.generate(row) }.join(",\n") + "\n  ]\n}\n"
File.write(ARGV.fetch(0), text)
warn "captured #{rows.size} runtime cases"
