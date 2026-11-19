# frozen_string_literal: true

# Pinned ukiryu/ukiryu ab6df353c6dc2d7faf7817a86aafcafa82579896, Ruby 4.0.1.
# bundle exec ruby capture_remainder.rb CRATE_ROOT remainder_inputs.json OUTPUT_JSON
# No network requests: the input set uses only bundled or local resources.
require 'ukiryu'
require 'json'
require 'yaml'
require 'date'
require 'json-schema'
crate, input, output = ARGV
raise 'oracle drift' unless Gem.loaded_specs.fetch('json-schema').version.to_s == '6.2.1'
Dir.chdir(crate)
rows = JSON.parse(File.read(input))
rows.each do |row|
 JSON::Validator.clear_cache
 i = row.fetch('input')
 begin
  value = YAML.safe_load(i.fetch('yaml'), permitted_classes: [Symbol, Date, Time])
  result = Ukiryu::SchemaValidator.validate_profile(value, schema_path: File.join(crate, i.fetch('schema')), strict: i.fetch('strict', false))
  row['expect'] = {'ok' => result}
 rescue StandardError => e
  row['expect'] = {'error' => {'class' => e.class.name, 'message' => e.message}}
 end
end
text = JSON.pretty_generate(rows).gsub("file://#{crate}", '${CRATE_FILE_URI}').gsub("Read of file at #{crate}", 'Read of file at ${CRATE_PATH}').gsub(crate, '${CRATE}') + "\n"
File.write(output, text)
warn "captured #{rows.size} rows"
