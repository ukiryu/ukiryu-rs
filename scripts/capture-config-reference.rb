# frozen_string_literal: true

# Regenerate the configuration transport reference from the pinned Ruby source.
# No gems are installed and no register/process execution occurs.
require 'json'
root = File.expand_path(ARGV.fetch(0))
revision = IO.popen(['git', '-C', root, 'rev-parse', 'HEAD'], &:read).strip
raise 'wrong reference revision' unless revision == 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
$LOAD_PATH.unshift(File.join(root, 'lib'))
require 'ukiryu/config'

def outcome
  { ok: yield }
rescue ArgumentError => error
  { error: { class: error.class.name, message: error.message } }
end

conversion = []
{
  timeout: [nil, '', '0', '010', '08', '0x10', '0x_ff', '0o17', '0b10', '0d10',
            '1_000', '1__0', '+12', '  -12 ', '0_10', '0x1_0', '-0x10', '0_9',
            "\u00a01", '999999999999999999999999999999999999999'],
  debug: [nil, '', 'true', 'TRUE', '1', 'yes', 'YES', 'false', '0', 'No', 'off', ' true '],
  shell: [nil, '', 'bash', 'BASH', ' strange shell '],
  format: [nil, '', 'json', 'YAML'],
  output: [nil, '', ' ', 'a b/Ω'],
  future: [nil, '', '123']
}.each do |attribute, values|
  values.each_with_index do |value, index|
    conversion << { id: "convert.#{attribute}.#{index}", attribute: attribute, input: value,
                    expect: outcome { Ukiryu::Config::TypeConverter.convert(attribute, value) } }
  end
end

environment = [
  {}, { 'UKIRYU_DEBUG' => '' }, { 'UKIRYU_TIMEOUT' => '010' },
  { 'UKIRYU_TIMEOUT' => '08' }, { 'UKIRYU_DEBUG' => 'false' },
  { 'UKIRYU_USE_COLOR' => 'true', 'NO_COLOR' => '' },
  { 'UKIRYU_USE_COLOR' => 'invalid', 'NO_COLOR' => '' },
  { 'UKIRYU_UNKNOWN' => 'true', 'UKIRYU_SHELL' => 'BASH' }
].each_with_index.map do |values, index|
  (Ukiryu::Config::EnvSchema.all_attributes.map { |key| Ukiryu::Config::EnvSchema.env_key(key) } + ['NO_COLOR']).each { |key| ENV.delete(key) }
  values.each { |key, value| ENV[key] = value }
  { id: "environment.#{index}", input: values,
    expect: outcome { Ukiryu::Config::EnvProvider.load_all } }
end

resolver = [
  { defaults: { x: true }, programmatic: { x: false } },
  { defaults: { x: true }, programmatic: { x: false }, env: { x: nil } },
  { defaults: { x: 1 }, programmatic: { x: 2 }, env: { x: 3 }, cli: { x: false } },
  { defaults: { x: nil } }, {}, { env: { x: '' } }
].each_with_index.map do |layers, index|
  instance = Ukiryu::Config::OverrideResolver.new(**layers)
  { id: "resolver.#{index}", input: layers, key: 'x',
    expect: { value: instance.resolve(:x), source: instance.source_for(:x) } }
end

(Ukiryu::Config::EnvSchema.all_attributes.map { |key| Ukiryu::Config::EnvSchema.env_key(key) } + ['NO_COLOR']).each { |key| ENV.delete(key) }
defaults = Ukiryu::Config.new.to_h
puts JSON.pretty_generate(schema: 'ukiryu-config-reference/1', source: revision,
                          ruby: RUBY_VERSION, profile: 'JSON transport; Ruby symbols have textual wire values',
                          conversions: conversion, environments: environment,
                          resolvers: resolver, defaults: defaults)
