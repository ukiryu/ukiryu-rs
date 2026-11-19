# frozen_string_literal: true

# Run with the pinned ukiryu/ukiryu ab6df353 bundle (Ruby 4.0.1,
# json-schema 6.2.1, date 3.6.0):
#   bundle exec ruby capture_date_time.rb CRATE_ROOT OUTPUT_JSON
# Output rows are appended to cases.json; provenance is recorded there.
require 'ukiryu'
require 'json'
require 'yaml'
require 'date'
crate, output = ARGV
schema = 'tests/schema_golden/inputs/synthetic/date_time.yaml'
raise 'json-schema oracle version changed' unless Gem.loaded_specs.fetch('json-schema').version.to_s == '6.2.1'
values = JSON.parse(File.read(File.join(__dir__, 'date_time_inputs.json'), encoding: 'UTF-8'))
raise 'empty date-time inputs' if values.empty?
rows = []
values.each_with_index do |value, i|
  # Quote scalars so YAML cannot turn an otherwise valid timestamp into Time
  # and bypass the string-only format validator.
  documents = { 'quoted' => "stamp: #{JSON.generate(value)}\n" }
  # Psych must keep these invalid 24-hour times as strings too.
  if [9, 37].include?(i)
    documents['unquoted'] = "stamp: #{value}\n"
  end
  documents.each do |style, text|
    [false, true].each do |strict|
      result = Ukiryu::SchemaValidator.validate_profile(
        YAML.safe_load(text, permitted_classes: [Symbol, Date, Time]),
        schema_path: File.join(crate, schema), strict: strict
      )
      rows << {
        'id' => "datetime.#{i}.#{style}.#{strict ? 'strict' : 'nonstrict'}",
        'op' => 'validate_profile',
        'input' => { 'yaml' => text, 'schema' => schema, 'strict' => strict },
        'expect' => { 'ok' => result }
      }
    end
  end
end
File.write(output, JSON.pretty_generate(rows) + "\n")
warn "rows=#{rows.size}"
