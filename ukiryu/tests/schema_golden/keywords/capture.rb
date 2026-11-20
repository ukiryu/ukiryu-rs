# frozen_string_literal: true
# Run in clean ukiryu ab6df353 with Ruby 4.0.1/json-schema 6.2.1:
# mise x -- bundle exec ruby /path/to/capture.rb OUTPUT
require 'ukiryu'
require 'json-schema'
require 'json'
require 'digest'
require 'open3'
revision,status=Open3.capture2('git','rev-parse','HEAD')
raise 'reference revision drift' unless status.success? && revision.strip == 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
status_text,status=Open3.capture2('git','status','--porcelain')
raise 'reference worktree is dirty' unless status.success? && status_text.empty?
raise 'oracle drift' unless RUBY_VERSION == '4.0.1' && Gem.loaded_specs.fetch('json-schema').version.to_s == '6.2.1'
rows = []
add = lambda do |id, schema, data, strict=false|
  JSON::Validator.clear_cache
  begin
    data = Ukiryu::SchemaValidator.send(:stringify_keys, data)
    result = JSON::Validator.fully_validate(schema, data, strict: strict)
    expect = {'ok'=>result}
  rescue StandardError => e
    expect = {'error'=>{'class'=>e.class.name,'message'=>e.message.gsub(Dir.pwd, '${CWD}')}}
  end
  rows << {'id'=>id,'schema'=>schema,'data'=>data,'strict'=>strict,'expect'=>expect}
end
# Every dialect/keyword is exercised with accepted and rejected documents.
[1,2,3,4,6].each do |draft|
  ['schema','hyper-schema'].each do |kind|
    prefix = "draft#{draft}.#{kind}"
    base = {'$schema'=>"http://json-schema.org/draft-0#{draft}/#{kind}#"}
    [1,{},[],nil,false,{'a'=>1}, {'a'=>'bad'},[1,2,3]].each_with_index do |data,i|
      [{'type'=>'integer'},{'properties'=>{'a'=>{'type'=>'integer','required'=>true}},'additionalProperties'=>false},
       {'items'=>{'type'=>'integer'},'minItems'=>2}, {'minimum'=>2,'maximum'=>4},
       {'required'=>['a']},{'disallow'=>'integer'},{'extends'=>{'type'=>'object'}},
       {'divisibleBy'=>2},{'multipleOf'=>2}, {'const'=>2}].each_with_index do |s,j|
        add.call("#{prefix}.#{i}.#{j}",base.merge(s),data)
      end
    end
  end
end
[nil,false,true,1,[],{},'garbage','http://json-schema.org/draft-07/schema#',"http://json-schema.org/draft-06/schema#\n",'http://a:bad/schema','http://[::1]/schema'].each_with_index do |v,i|
 add.call("schema.malformed.#{i}",{'$schema'=>v,'type'=>'object'}, {})
end
samples=[nil,false,true,1,1.5,'x',[],{},[1,1.0,{},[]],{'a'=>1}]
%w[definitions properties patternProperties required enum allOf anyOf oneOf not extends const uniqueItems additionalItems multipleOf dependencies propertyNames disallow minimum minLength pattern].each do |key|
 samples.each_with_index do |v,i|
  [nil,1,'abc',[],[1,1],{}, {'a'=>1,'b'=>'bad'}].each_with_index do |data,j|
   schema={key=>v}; schema['items']=[{'type'=>'integer'}] if key=='additionalItems'
   add.call("malformed.#{key}.#{i}.#{j}",schema,data)
  end
 end
end
%w[const uniqueItems multipleOf divisibleBy].each do |key|
 [nil,false,true,0,2,2.5,[],{},[1,2],{'a'=>1}].each_with_index do |v,i|
  [nil,false,0,1,2,4,5,2.5,5.0,[],[1,1],[1,1.0],[{},{}],['x','y'],{'a'=>1}].each_with_index do |data,j|
   add.call("keyword.#{key}.#{i}.#{j}",{key=>v},data)
  end
 end
end
[
 {'patternProperties'=>{'^a'=>{'type'=>'integer'},'a$'=>{'minimum'=>2}},'additionalProperties'=>false},
 {'properties'=>{'a'=>{'type'=>'integer'}},'patternProperties'=>{'^a'=>{'minimum'=>2},'^b'=>{'type'=>'string'}}},
 {'properties'=>{'a'=>{}},'patternProperties'=>{'^b'=>{}},'additionalProperties'=>false},
 {'patternProperties'=>{'['=>{}}},
 {'dependencies'=>{'a'=>['b','c']}}, {'dependencies'=>{'a'=>'b'}},
 {'dependencies'=>{'a'=>{'required'=>['b']}}},
 {'propertyNames'=>{'pattern'=>'^a'}}, {'propertyNames'=>{'type'=>'integer'}},
 {'items'=>[{'type'=>'integer'}],'additionalItems'=>false},
 {'items'=>[{'type'=>'integer'}],'additionalItems'=>{'type'=>'string'}},
 {'extends'=>[{'properties'=>{'a'=>{}}}],'additionalProperties'=>false},
 {'$schema'=>'http://json-schema.org/draft-03/schema#','extends'=>[{'properties'=>{'a'=>{}}}],'additionalProperties'=>false}
].each_with_index do |s,i|
 [{},{'a'=>1},{'a'=>2,'b'=>'ok'},{'a'=>1,'b'=>2,'c'=>false},{'c'=>1},[],[1],[1,'ok'],[1,2]].each_with_index do |d,j|
  [false,true].each{|strict| add.call("objects.#{i}.#{j}.#{strict}",s,d,strict)}
 end
end
['[','(','\\','(?<x>)\\k<y>','a{4,2}','\\uZZZZ','^a','\\p{L}+','[/','a/(',"(\n",'\\u{110000}','\\x'].each_with_index do |pattern,i|
 add.call("regexp.#{i}",{'pattern'=>pattern},'abc')
end
["abc\ndef","http://a\nhttp://b","\nhttp://a","http://a\n", "http://a\r\n", "x\ny:z",'http://a:bad',"http://a b\nhttp://b", "http://a\n b"].each_with_index do |v,i|
 add.call("uri.multiline.#{i}",{'format'=>'uri'},v)
end
# Dialect distinctions and composition ordering need their own probes.
[1,2,3,4,6].each do |draft|
 base={'$schema'=>"http://json-schema.org/draft-0#{draft}/schema#"}
 [ {'properties'=>{'a'=>{}}}, {'properties'=>{'a'=>{'optional'=>true}}},
   {'properties'=>{'a'=>{'optional'=>false,'required'=>false}}},
   {'properties'=>{'a'=>{'requires'=>'b'}}},
   {'minimum'=>2,'minimumCanEqual'=>false}, {'maximum'=>2,'maximumCanEqual'=>false},
   {'minimum'=>2,'exclusiveMinimum'=>true}, {'maximum'=>2,'exclusiveMaximum'=>true},
   {'patternProperties'=>{'^a'=>{'type'=>'integer'}}},
   {'dependencies'=>{'a'=>'b'}}, {'propertyNames'=>{'pattern'=>'^a'}},
   {'items'=>[{'type'=>'integer'}],'additionalItems'=>false},
   {'type'=>[{'type'=>'integer'},'null']}, {'disallow'=>[{'minimum'=>2},'null']}
 ].each_with_index do |schema,i|
  [nil,1,2,3,{}, {'a'=>1}, {'a'=>1,'b'=>2}, {'b'=>1},[],[1,2]].each_with_index do |data,j|
   [false,true].each{|strict|add.call("dialect.#{draft}.#{i}.#{j}.#{strict}",base.merge(schema),data,strict)}
  end
 end
end
['http://bad host/schema','1bad:x','http://','http://a:bad/schema',
 'HTTP://json-schema.org/draft-06/schema#','http://JSON-SCHEMA.ORG/draft-06/schema#',
 'http://u:p@json-schema.org:123/draft-06/schema?foo#bar',
 "http://json-schema.org/draft-06/schema#\n", "http://json-schema.org/draft-06/schema\n",'//json-schema.org/draft-06/schema',
 ':x',':','http://[%]/schema','http://[😀]/schema','http://a:%FF/',
 'http://a<b/schema','http://[bad!]/schema','http://[]/schema','http://a:%3f/schema'].each_with_index do |v,i|
 add.call("schema.uri.#{i}",{'$schema'=>v,'type'=>'integer'},'abc')
end
%w[allOf anyOf oneOf].each do |keyword|
 [{}, {'x'=>{}}, [false], [[]], ['x'], [{'type'=>'integer'},{'minimum'=>2}]].each_with_index do |value,i|
  [1,2,'abc',{}].each_with_index{|data,j| add.call("composition.#{keyword}.#{i}.#{j}",{keyword=>value},data)}
 end
end
resource=File.expand_path('resources/target.json',__dir__)
[resource, [resource], [1,resource], 'ukiryu-keywords-missing-schema.json', ['ukiryu-keywords-missing-schema.json']].each_with_index do |extends,i|
 [1,2,'abc',{}].each_with_index{|data,j| add.call("extends.resource.#{i}.#{j}",{'id'=>'urn:ukiryu:keyword-root','extends'=>extends},data)}
end
[resource, 'file://'+resource, 'ukiryu-keywords-missing-schema.json'].each_with_index do |extends,i|
 [1,2,'abc',{}].each_with_index{|data,j| add.call("extends.file.#{i}.#{j}",{'id'=>'file://'+File.expand_path('root.json',__dir__),'extends'=>extends},data)}
end
['2','0.1','1e2','1_0','1__0','NaN','Infinity','-Infinity','+Infinity',' 2 ','+.5','2foo','1.','1_','1d2'].each_with_index do |divisor,i|
 [0,1,2,10,0.3].each_with_index{|data,j| add.call("decimal.#{i}.#{j}",{'multipleOf'=>divisor},data)}
end
%w[.nan .inf -.inf].each_with_index do |yaml,i|
 add.call("decimal.nonfinite.#{i}",{'multipleOf'=>2},YAML.safe_load(yaml))
 rows.last.delete('data');rows.last['data_yaml']=yaml
end
out={'reference'=>{'repository'=>'ukiryu/ukiryu','revision'=>'ab6df353c6dc2d7faf7817a86aafcafa82579896','ruby'=>RUBY_VERSION,'json-schema'=>Gem.loaded_specs.fetch('json-schema').version.to_s,'addressable'=>Gem.loaded_specs.fetch('addressable').version.to_s,'generator_sha256'=>Digest::SHA256.file(__FILE__).hexdigest,'resource_sha256'=>Digest::SHA256.file(resource).hexdigest},'cases'=>rows}
# One compact case per line keeps the generated evidence reviewable.
text = "{\n  \"reference\": " + JSON.generate(out.fetch('reference')) + ",\n  \"cases\": [\n" + rows.map{|row| '    '+JSON.generate(row)}.join(",\n") + "\n  ]\n}\n"
File.write(ARGV.fetch(0),text.gsub('file://'+File.expand_path(__dir__), '${FIXTURE_FILE_URI}').gsub(File.expand_path(__dir__), '${FIXTURE}'))
warn "captured #{rows.size} cases"
