# frozen_string_literal: true
require 'ukiryu'
require 'json-schema'
require 'json'
require 'socket'
require 'digest'
require 'timeout'
require 'open3'
root = ARGV.fetch(0)
revision = Open3.capture2('git', '-C', root, 'rev-parse', 'HEAD').first.strip
raise 'oracle drift' unless revision == 'ab6df353c6dc2d7faf7817a86aafcafa82579896' && Open3.capture2('git', '-C', root, 'status', '--porcelain').first.empty?
raise 'loaded oracle mismatch' unless File.realpath(Gem.loaded_specs.fetch('ukiryu').full_gem_path) == File.realpath(root)
raise 'oracle runtime drift' unless RUBY_ENGINE == 'ruby' && RUBY_VERSION == '4.0.1' && Gem.loaded_specs.fetch('json-schema').version.to_s == '6.2.1'
server = TCPServer.new('127.0.0.1', 0)
base = "http://127.0.0.1:#{server.addr[1]}"
refused = TCPServer.new('127.0.0.1', 0)
dead = "http://127.0.0.1:#{refused.addr[1]}"
refused.close
routes = {
 '/integer.json' => [200, '{"type":"integer"}'],
 '/dir/root.json' => [200, '{"$ref":"../integer.json"}'],
 '/dir/id.json' => [200, '{"id":"nested/entry.json","$ref":"../../integer.json"}'],
 '/fragment.json' => [200, '{"definitions":{"n":{"type":"integer"}}}'],
 '/redirect' => [302, '', '/dir/root.json'],
 '/redir/from/start' => [302, '', '/dir/root.json'],
 '/redirect-missing' => [302, '', '/missing'],
 '/fragment-ref.json' => [200, '{"$ref":"fragment.json#/definitions/n"}'],
 '/definition.yaml' => [200, "type: object\nrequired: [name]\n"],
 '/invalid.yaml' => [200, "x: [\n"],
 '/false.yaml' => [200, 'false'],
 '/symbol.yaml' => [200, ':foo'],
}
requests = []
thread = Thread.new do
 loop do
  c = server.accept
  line = c.gets
  next c.close unless line
  path = line.split[1]
  requests << path
  while (h = c.gets) && h != "\r\n"; end
  status, body, location = routes.fetch(path, [404, 'missing'])
  c.write "HTTP/1.1 #{status} #{ {200=>'OK',302=>'Found',404=>'Not Found'}[status]}\r\nContent-Length: #{body.bytesize}\r\nConnection: close\r\n#{location ? "Location: #{base}#{location}\r\n" : ''}\r\n#{body}"
  c.close
 end
end
rows=[]
record = lambda do |id, input, &action|
 JSON::Validator.clear_cache
 begin
  result=Timeout.timeout(10, &action)
  expected={'ok'=>result}
 rescue StandardError => e
  expected={'error'=>{'class'=>e.class.name,'message'=>e.message}}
 end
 rows << {'id'=>id,'input'=>input,'expect'=>expected}
end
[1, true, [], {}, nil, false, "#{base}/integer.json\n"].each_with_index do |ref,i|
 schema={'$ref'=>ref}
 record.call("ref.shape.#{i}",{'op'=>'schema','schema'=>schema,'data'=>{}}){JSON::Validator.fully_validate(schema,{})}
end
[5, 1.5, true, false, nil, 'abc'].each_with_index do |value, i|
 schema={'definitions'=>{'x'=>value},'$ref'=>'#/definitions/x/a'}
 record.call("ref.scalar.#{i}",{'op'=>'schema','schema'=>schema,'data'=>{}}){JSON::Validator.fully_validate(schema,{})}
end
['#/definitions/x/a/z', '#/definitions/x/z', '#/definitions/x/b/b'].each_with_index do |ref,i|
 schema={'definitions'=>{'x'=>'abc'},'$ref'=>ref}
 record.call("ref.string.#{i}",{'op'=>'schema','schema'=>schema,'data'=>{}}){JSON::Validator.fully_validate(schema,{})}
end
['http://bad host/schema','1bad:foo','http://127.0.0.1:abc/x','http:','http://[bad!]/x','http://[bad/]/x',':ambiguous'].each_with_index do |uri,i|
 schema={'$ref'=>uri}
 record.call("uri.invalid.#{i}",{'op'=>'schema','schema'=>schema,'data'=>{}}){JSON::Validator.fully_validate(schema,{})}
end
[1,'x'].each do |data|
 document=JSON.dump(data)
 %w[/integer.json /dir/root.json /dir/id.json /fragment.json#/definitions/n /fragment-ref.json /redirect /redir/from/start /redirect-missing /missing].each do |path|
  schema=base+path
  record.call("schema.#{path}.#{data}",{'op'=>'schema','schema'=>schema,'data'=>document}) { JSON::Validator.fully_validate(schema,document) }
 end
 schema={'id'=>base+'/dir/root.json', '$ref'=>'../integer.json'}
 record.call("inline.relative.#{data}",{'op'=>'schema','schema'=>schema,'data'=>document}){JSON::Validator.fully_validate(schema,document)}
end
[base+'/missing',dead+'/schema.json'].each do |url|
 record.call(url.start_with?(dead) ? 'read.refused' : 'read.404',{'op'=>'schema','schema'=>{'$ref'=>url},'data'=>{}}){JSON::Validator.fully_validate({'$ref'=>url},{})}
end
klass=Ukiryu::Definition::DefinitionValidator
klass.send(:remove_const,:REMOTE_SCHEMA_URL)
%w[/definition.yaml /invalid.yaml /false.yaml /symbol.yaml /missing].each do |path|
 klass.const_set(:REMOTE_SCHEMA_URL,base+path)
 record.call("download.#{path}",{'op'=>'download','url'=>base+path}){klass.send(:download_and_load_schema)}
 record.call("fallback.#{path}",{'op'=>'fallback','url'=>base+path,'data'=>{}}){klass.validate({}).to_h}
 klass.send(:remove_const,:REMOTE_SCHEMA_URL)
end
klass.const_set(:REMOTE_SCHEMA_URL,dead+'/schema.yaml')
record.call('download.refused',{'op'=>'download','url'=>dead+'/schema.yaml'}){klass.send(:download_and_load_schema)}
record.call('fallback.refused',{'op'=>'fallback','url'=>dead+'/schema.yaml','data'=>{}}){klass.validate({}).to_h}
text=JSON.pretty_generate({'generator_sha256'=>Digest::SHA256.file(__FILE__).hexdigest,'oracle_state'=>'clean','oracle'=>revision,'ruby'=>RUBY_VERSION,'json-schema'=>Gem.loaded_specs.fetch('json-schema').version.to_s,'lock_sha256'=>Digest::SHA256.file(File.join(root,'Gemfile.lock')).hexdigest,'routes'=>routes,'cases'=>rows,'requests'=>requests})
File.write(ARGV.fetch(1),text.gsub(base,'${BASE}').gsub(dead,'${DEAD}').gsub(dead.delete_prefix('http://'), '${DEAD_AUTH}').gsub("port #{dead.split(':').last}", 'port ${DEAD_PORT}')+"\n")
warn "captured #{rows.size} rows, #{requests.size} local requests"
thread.kill;thread.join;server.close
