# frozen_string_literal: true

# Run from a clean, pinned oracle with its bundle activated:
# bundle exec ruby /path/to/capture_source_examples.rb OUT.json
require 'json'
require 'open3'
require 'tmpdir'
require 'digest'
require 'rbconfig'
output = File.expand_path(ARGV.fetch(0))
reference = File.realpath(ARGV.fetch(1, Dir.pwd))
Dir.chdir(reference)
$LOAD_PATH.unshift(File.join(reference, 'lib'))
require 'ukiryu'

PIN = 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
SPECS = %w[executor_spec executor_stdin_spec cli_stdin_spec].map { |n| "spec/ukiryu/#{n}.rb" }.freeze
sha, status = Open3.capture2('git', 'rev-parse', 'HEAD')
abort 'wrong oracle revision' unless status.success? && sha.strip == PIN
clean, status = Open3.capture2('git', 'status', '--porcelain')
abort 'oracle must be clean' unless status.success? && clean.empty?
loaded = $LOADED_FEATURES.find { |f| f.end_with?('/lib/ukiryu.rb') }
abort 'ukiryu must be loaded from this oracle' unless File.realpath(loaded) == File.realpath('lib/ukiryu.rb')

out = {
  'oracle' => PIN,
  'ruby' => RUBY_VERSION, 'engine' => RUBY_ENGINE, 'host' => RUBY_PLATFORM,
  'generator_commit' => Open3.capture2('git', '-C', File.dirname(__FILE__), 'rev-parse', 'HEAD').first.strip,
  'generator_sha256' => Digest::SHA256.file(__FILE__).hexdigest,
  'spec_sha256' => SPECS.to_h { |f| [f, Digest::SHA256.file(f).hexdigest] }
}
Dir.mktmpdir('ukiryu-example-census') do |dir|
  report = File.join(dir, 'examples.json')
  _, stderr, status = Open3.capture3(RbConfig.ruby, Gem.bin_path('rspec-core', 'rspec'), *SPECS,
                                    '--dry-run', '--format', 'json', '--out', report)
  abort stderr unless status.success?
  data = JSON.parse(File.read(report))
  abort 'empty or incomplete census' unless data.dig('summary', 'example_count') == 53
  out['examples'] = data.fetch('examples').map { |e| e.slice('id', 'full_description') }
end

# A real IO.pipe, not StringIO; the producer closes its write end at EOF.
reader, writer = IO.pipe
producer = Thread.new do
  writer.write('pipe')
  writer.write(' data')
ensure
  writer.close
end
begin
  result = Ukiryu::Executor.execute('/bin/cat', [], stdin: reader, shell: :bash, timeout: 90)
  out['pipe'] = { 'stdout_hex' => result.stdout.b.unpack1('H*'),
                  'stderr_hex' => result.stderr.b.unpack1('H*'), 'exit_status' => result.exit_status }
ensure
  reader.close
  producer.join
end

out['commands'] = [
  ['echo', ['hello', 'world']],
  ['/usr/bin/echo', ['hello world', "test's", '$HOME; & | *']]
].map do |executable, args|
  result = Ukiryu::Executor.execute(executable, args, env: {}, shell: :bash, timeout: 30)
  { 'executable' => executable, 'arguments' => args,
    'command' => Ukiryu::Executor.build_command(executable, args, Ukiryu::Shell::Bash),
    'stdout_hex' => result.stdout.b.unpack1('H*'), 'stderr_hex' => result.stderr.b.unpack1('H*'),
    'exit_status' => result.exit_status }
end

Dir.mktmpdir('ukiryu-lookup') do |dir|
  first, extra = %w[path extra].map { |n| File.join(dir, n).tap { |p| Dir.mkdir(p) } }
  [[first, 'common'], [extra, 'common'], [extra, 'extra_only']].each do |d, name|
    file = File.join(d, name)
    File.write(file, "#!/bin/sh\nexit 0\n")
    File.chmod(0o755, file)
  end
  paths = Ukiryu::Platform.executable_search_paths
  saved = paths.dup
  begin
    paths.replace([first])
    out['lookup'] = %w[common absent extra_only].map do |name|
      found = Ukiryu::Executor.find_executable(name, additional_paths: [extra])
      { 'command' => name, 'found' => found&.sub(dir, '<ROOT>') }
    end
  ensure
    paths.replace(saved)
  end
end
File.write(output, JSON.pretty_generate(out) + "\n")
