# frozen_string_literal: true
# Run in the same pinned clean Ruby reference as capture.rb.
require 'ukiryu'
require 'json-schema'
require 'json'
require 'digest'
require 'tmpdir'
require 'open3'
revision, status = Open3.capture2('git', 'rev-parse', 'HEAD')
raise 'reference revision drift' unless status.success? && revision.strip == 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
dirty, status = Open3.capture2('git', 'status', '--porcelain')
raise 'dirty reference' unless status.success? && dirty.empty?
raise 'oracle drift' unless RUBY_VERSION == '4.0.1' && Gem.loaded_specs.fetch('json-schema').version.to_s == '6.2.1'
rows = []
Dir.mktmpdir('ukiryu-extends') do |dir|
  Dir.chdir(dir) do
    %w[urn custom].each do |scheme|
      path = "#{scheme}:ukiryu-schema"
      [nil, '{"type":"integer"}', '{}', 'false', '[]', '1', 'invalid'].each do |contents|
        File.write(path, contents) if contents
        begin
          JSON::Validator.clear_cache
          result = JSON::Validator.fully_validate({'id'=>'urn:root', 'extends'=>path}, 1)
          expect = {'ok'=>result}
        rescue StandardError => e
          expect = {'error'=>{'class'=>e.class.name, 'message'=>e.message.gsub(path, '${RESOURCE}')}}
        end
        rows << {'scheme'=>scheme, 'contents'=>contents, 'expect'=>expect}
        File.unlink(path) if contents
      end
    end
  end
end
out = {'reference'=>{'revision'=>revision.strip, 'ruby'=>RUBY_VERSION,
       'json-schema'=>Gem.loaded_specs.fetch('json-schema').version.to_s,
       'addressable'=>Gem.loaded_specs.fetch('addressable').version.to_s,
       'generator_sha256'=>Digest::SHA256.file(__FILE__).hexdigest}, 'cases'=>rows}
File.write(ARGV.fetch(0), JSON.pretty_generate(out) + "\n")
warn "captured #{rows.size} local URI-named extends cases"
