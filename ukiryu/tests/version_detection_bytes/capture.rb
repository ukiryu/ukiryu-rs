# frozen_string_literal: true
require 'bundler/setup'
require 'ukiryu'
require 'json'
require 'digest'
require 'open3'
source=Gem.loaded_specs.fetch('ukiryu').full_gem_path
commit,status=Open3.capture2('git','-C',source,'rev-parse','HEAD')
abort 'unexpected oracle' unless status.success? && commit.strip=='ab6df353c6dc2d7faf7817a86aafcafa82579896' && RUBY_VERSION=='4.0.1'
lock=Digest::SHA256.file(ENV.fetch('BUNDLE_GEMFILE')+'.lock').hexdigest
abort 'unexpected lock' unless lock=='fc3b12f2ede259028b638d2f26c65ddf9506d142c20a8bf0c1b151741078b65e'
outputs=['tool 1.0',"tool \xFF\xFE 1.0".b,'Ω 1.0',"\uFEFF1.0","1\0.0","\xFF".b,"x\xF0\x9F 1.0".b,"\xC0\xAF 1.0".b,'é1.0',"a\n1.0",'']
patterns=['([\d.]+)','(.)','(.*)','(Ω)','(\p{L}+)','(\xFF)','[','(?<x>a)(', '(a)\\1', '(?i)(é)', '(\u{FF})', 'a/(', "a\n(","a\t(", '\/(', '(?<=tool )(\d+\.\d+)', '(?<one>1)(?<two>\.0)', '\A(.*)\z', '^', '(x)?1.0']
rows=[]
outputs.each_with_index do |raw,n|
 %w[UTF-8 ASCII-8BIT US-ASCII].each do |encoding|
  output=raw.b.dup.force_encoding(encoding)
  patterns.each_with_index do |pattern,p|
   input={output:{hex:output.b.unpack1('H*'),encoding:encoding},pattern:pattern}
   begin
    result=Ukiryu::Tool::Loader.send(:extract_version_from_pattern,output,pattern)
    expect={ok:result && {hex:result.b.unpack1('H*'),encoding:result.encoding.name}}
   rescue=>e
    stage='compile'
    begin
     scrubbed=output.scrub('');compiled=Regexp.new(pattern);stage='match';scrubbed.match(compiled)
    rescue=>again
     abort 'stage replay differs' unless [again.class.name,again.message]==[e.class.name,e.message]
    end
    expect={error:{class:e.class.name,message:e.message,stage:stage}}
   end
   rows << {id:"version-bytes.#{n}.#{encoding}.#{p}",input:input,expect:expect}
  end
 end
end
[{output:nil,pattern:'['},{output:{hex:'ff',encoding:'ASCII-8BIT'},pattern:nil},{output:nil,pattern:nil}].each_with_index do |input,i|
 raw=input[:output] && [input[:output][:hex]].pack('H*').force_encoding(input[:output][:encoding])
 result=Ukiryu::Tool::Loader.send(:extract_version_from_pattern,raw,input[:pattern])
 abort 'nil short circuit changed' unless result.nil?
 rows << {id:"version-bytes.nil.#{i}",input:input,expect:{ok:nil}}
end
path=File.join(__dir__,'cases.json');File.write(path,JSON.pretty_generate(rows)+"\n")
provenance={oracle_commit:commit.strip,ruby:RUBY_VERSION,lock_sha256:lock,source:'lib/ukiryu/tool/loader.rb',source_sha256:Digest::SHA256.file(File.join(source,'lib/ukiryu/tool/loader.rb')).hexdigest,rows:rows.size,sha256:Digest::SHA256.file(path).hexdigest}
File.write(File.join(__dir__,'provenance.json'),JSON.pretty_generate(provenance)+"\n");puts JSON.generate(provenance)
