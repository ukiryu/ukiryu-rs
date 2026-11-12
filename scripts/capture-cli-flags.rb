require 'bundler/setup'
require File.join(File.expand_path(ARGV.fetch(0)), 'lib/ukiryu')
require 'json'
require 'stringio'
require 'digest'
class Ukiryu::CliCommands::RunCommand
  def initialize(options)
    @options = options
  end
  def run(*args)
    puts JSON.generate(arguments: args, options: @options)
  end
end
rows = []
[
  %w[exec tool],
  %w[exec tool --dry-run],
  %w[exec tool --no-dry-run],
  %w[exec tool --dry-run --no-dry-run],
  %w[exec tool --no-dry-run --dry-run],
  %w[exec tool --no-stdin --no-raw],
  %w[exec tool --dry-run=false],
  %w[exec tool -- action --raw x=1],
  %w[exec tool --unknown],
].each do |args|
  original_out, original_err = $stdout, $stderr
  $stdout, $stderr = StringIO.new, StringIO.new
  begin
    Ukiryu::Cli.start(args)
    rows << {argv: args, stdout: $stdout.string, stderr: $stderr.string}
  rescue Exception => error
    rows << {argv: args, stdout: $stdout.string, stderr: $stderr.string, error: error.class.name, message: error.message}
  ensure
    $stdout, $stderr = original_out, original_err
  end
end
source_commit = IO.popen(['git', '-C', File.expand_path(ARGV.fetch(0)), 'rev-parse', 'HEAD'], &:read).strip
lock_sha256 = Digest::SHA256.file(Bundler.default_lockfile).hexdigest
puts JSON.pretty_generate(source_commit: source_commit, lock_sha256: lock_sha256, ruby: RUBY_VERSION, thor: Gem.loaded_specs.fetch('thor').version.to_s, rows: rows)
