# frozen_string_literal: true

# Capture the Register behaviour reference from the pinned Ruby source.
#
#   ruby fixtures/register/capture-reference.rb <ruby-reference> > fixtures/register/reference.json
#
# Every scenario runs against fixtures/register/spec (a verbatim copy of the
# pinned spec/fixtures/register tree) or against scratch directories this
# script creates itself under a fresh Dir.mktmpdir. The network is never used:
# Register::REMOTE_URL is redirected to a synthetic local git repository, and
# the user's real home directory is never consulted (HOME is a scratch path).
require 'json'
require 'tmpdir'
require 'fileutils'
require 'open3'

REVISION = 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
root = File.expand_path(ARGV.fetch(0))
revision = IO.popen(['git', '-C', root, 'rev-parse', 'HEAD'], &:read).strip
raise "wrong reference revision #{revision}" unless revision == REVISION

$LOAD_PATH.unshift(File.join(root, 'lib'))
require 'ukiryu'

FIXTURE = File.expand_path('spec', __dir__)
SOURCE_FIXTURE = File.join(root, 'spec', 'fixtures', 'register')
Register = Ukiryu::Register

def tree(dir)
  Dir.glob('**/*', File::FNM_DOTMATCH, base: dir).reject { |p| File.directory?(File.join(dir, p)) }
     .sort.to_h { |p| [p, File.binread(File.join(dir, p))] }
end
raise 'fixtures/register/spec differs from pinned spec/fixtures/register' unless tree(FIXTURE) == tree(SOURCE_FIXTURE)

# Typed YAML scalar so value types (String vs Float vs Integer) survive JSON.
def typed(value)
  case value
  when nil then nil
  when String then { 'type' => 'string', 'value' => value }
  when Integer then { 'type' => 'integer', 'value' => value.to_s }
  when Float then { 'type' => 'float', 'value' => value.to_s }
  when true, false then { 'type' => 'boolean', 'value' => value }
  when Symbol then { 'type' => 'symbol', 'value' => value.to_s }
  else { 'type' => value.class.name, 'value' => value.inspect }
  end
end

$tokens = {}
def rel(path)
  return nil if path.nil?

  s = path.to_s
  $tokens.sort_by { |k, _| -k.length }.each do |prefix, token|
    return token + s[prefix.length..] if s.start_with?(prefix)
  end
  s
end

def outcome
  { 'ok' => yield }
rescue StandardError => e
  { 'error' => { 'class' => e.class.name, 'message' => rel_message(e.message) } }
end

def rel_message(message)
  $tokens.sort_by { |k, _| -k.length }.reduce(message.to_s) do |m, (prefix, token)|
    m.gsub(prefix, token).gsub(prefix.tr('/', '\\'), token)
  end
end

def git(*args, chdir:)
  out, status = Open3.capture2e('git', '-c', 'user.name=Ukiryu Fixture', '-c', 'user.email=fixture@example.invalid',
                                '-c', 'init.defaultBranch=main', '-c', 'commit.gpgsign=false', *args, chdir: chdir)
  raise "git #{args.join(' ')} failed: #{out}" unless status.success?

  out
end

def write(path, content)
  FileUtils.mkdir_p(File.dirname(path))
  File.write(path, content)
end

def action_names(actions)
  case actions
  when nil then nil
  when Hash then actions.keys.map(&:to_s)
  else actions.map { |a| a.is_a?(Hash) ? (a[:name] || a['name']).to_s : a.class.name }
  end
end

result = { 'source_revision' => REVISION }

# ---------------------------------------------------------------- instance API
$tokens[FIXTURE] = '<FIXTURE>'
reg = Register.at(FIXTURE)
names = reg.tool_names
version_queries = [nil, '1.0', '6.9', '7.1', '10.0', '0.92', '2024.01.17', 'missing']
result['fixture'] = {
  'source' => reg.source.to_s,
  'exists' => reg.exists?,
  'valid' => reg.valid?,
  'tool_names' => names,
  'tool?' => (names + %w[missing interfaces]).to_h { |n| [n, reg.tool?(n)] },
  'list_versions' => (names + ['missing']).to_h do |n|
    [n, reg.list_versions(n).map { |path, eq| [rel(path), typed(eq)] }]
  end,
  'load_tool_yaml' => (names + ['missing']).to_h do |n|
    by_content = reg.list_versions(n).keys.to_h { |p| [File.read(p), rel(p)] }
    [n, version_queries.to_h do |v|
      content = reg.load_tool_yaml(n, version: v)
      [v || '<latest>', content.nil? ? nil : (by_content[content] || '<other>')]
    end]
  end,
  'implementation_index' => (names + ['missing']).to_h do |n|
    idx = reg.load_implementation_index(n)
    [n, idx && {
      'name' => typed(idx.name), 'interface' => typed(idx.interface),
      'interface_version' => typed(idx.interface_version),
      'implementations' => idx.implementations.map do |impl|
        { 'name' => typed(impl[:name]), 'default' => typed(impl[:default]),
          'versions' => (impl[:versions] || []).map { |vs| vs.transform_keys(&:to_s).transform_values { |x| x.is_a?(Hash) ? x.transform_keys(&:to_s) : typed(x) } } }
      end
    }]
  end,
  'interface' => (Dir.children(File.join(FIXTURE, 'interfaces')).map { |f| File.basename(f, '.yaml') }.sort + ['missing']).to_h do |n|
    i = reg.load_interface(n)
    [n, i && { 'name' => typed(i.name), 'version' => typed(i.version), 'actions' => action_names(i.actions) }]
  end,
  'implementation_version' => names.flat_map do |n|
    reg.list_versions(n).keys.map do |p|
      parts = p.delete_prefix("#{FIXTURE}/tools/#{n}/").split('/', 2)
      v = reg.load_implementation_version(n, parts[0], parts[1])
      { 'tool' => n, 'implementation' => parts[0], 'file' => parts[1],
        'loaded' => v && { 'implements' => typed(v.implements), 'version' => typed(v.version),
                           'display_name' => typed(v.display_name),
                           'profiles' => v.execution_profiles.map { |pr| (pr[:name] || pr['name']).to_s } } }
    end
  end,
  'implementation_version_missing' => reg.load_implementation_version('gzip', 'default', 'missing.yaml')
}

# ------------------------------------------------------- structure + discovery
Dir.mktmpdir('ukiryu-register-oracle') do |tmp|
  tmp = File.realpath(tmp).tr('\\', '/')
  $tokens[tmp] = '<TMP>'
  layouts = {
    'missing' => nil,
    'empty' => {},
    'tools_empty' => { 'tools/' => nil },
    'tool_dir_no_yaml' => { 'tools/demo/readme.txt' => 'x' },
    'top_level_yaml_only' => { 'tools/demo.yaml' => 'name: demo' },
    'nested_too_deep' => { 'tools/demo/default/1.0.yaml' => 'version: "1.0"' },
    'index_only' => { 'tools/demo/index.yaml' => "name: demo\ninterface: demo\nimplementations: []\n" },
    'other_yaml_only' => { 'tools/demo/1.0.yaml' => 'name: demo' },
    'file_not_dir' => :file
  }
  result['structure'] = layouts.to_h do |name, spec|
    dir = File.join(tmp, 'layout', name)
    case spec
    when nil then nil
    when :file then write(dir, 'not a directory')
    else
      FileUtils.mkdir_p(dir)
      spec.each { |p, c| c.nil? ? FileUtils.mkdir_p(File.join(dir, p)) : write(File.join(dir, p), c) }
    end
    r = Register.at(dir)
    [name, { 'exists' => r.exists?, 'valid' => r.valid?, 'tool_names' => outcome { r.tool_names } }]
  end

  # Ordering, duplicates and value types of list_versions / load_tool_yaml.
  dup = File.join(tmp, 'dup')
  write(File.join(dup, 'tools/dup/index.yaml'), <<~YAML)
    name: dup
    interface: dup
    implementations:
    - name: zeta
      versions:
      - equals: '2.0'
        file: 2.0.yaml
      - equals: 1.5
        file: 1.5.yaml
      - equals: 3
        file: 3.yaml
      - after: '4.0'
        file: 4.yaml
      - equals: '2.0-again'
        file: 2.0.yaml
      - file: nofile-equals.yaml
      - equals: '9'
    - name: alpha
      versions:
      - equals: '1.0'
        file: 1.0.yaml
    - name: noversions
    - name: zeta
      versions:
      - equals: 'z2'
        file: 2.0.yaml
  YAML
  %w[zeta/2.0.yaml zeta/1.5.yaml zeta/3.yaml zeta/4.yaml alpha/1.0.yaml].each do |f|
    write(File.join(dup, 'tools/dup', f), "file: #{f}\n")
  end
  write(File.join(dup, 'tools/dup/1.5.yaml'), "file: top-level 1.5\n")
  write(File.join(dup, 'tools/empty/index.yaml'), '')
  write(File.join(dup, 'tools/nullidx/index.yaml'), "--- ~\n")
  write(File.join(dup, 'tools/noimpls/index.yaml'), "name: noimpls\ninterface: x\n")
  write(File.join(dup, 'interfaces/nested/x/1.0.yaml'), "name: nested\nversion: '1.0'\nactions: {}\n")
  write(File.join(dup, 'interfaces/empty.yaml'), '')
  r = Register.at(dup)
  result['ordering'] = {
    'tool_names' => r.tool_names,
    'list_versions' => %w[dup empty nullidx noimpls].to_h { |n| [n, outcome { r.list_versions(n).map { |p, eq| [rel(p), typed(eq)] } }] },
    'load_tool_yaml' => [nil, '2.0', '1.5', '3', '2.0-again', 'z2', '4', 'missing'].to_h do |v|
      [v || '<latest>', outcome { r.load_tool_yaml('dup', version: v) }]
    end,
    'load_tool_yaml_empty' => outcome { r.load_tool_yaml('empty') },
    'interface_nested' => outcome { (i = r.load_interface('nested/x/1.0')) && typed(i.name) },
    'interface_empty' => outcome { r.load_interface('empty') },
    'index_empty' => outcome { r.load_implementation_index('empty') },
    'index_null' => outcome { r.load_implementation_index('nullidx') },
    'index_noimpls' => outcome { (i = r.load_implementation_index('noimpls')) && { 'implementations' => i.implementations.size, 'interface_version' => typed(i.interface_version) } }
  }

  # Discovery precedence: ENV -> dev -> user; Ruby derives dev from __FILE__,
  # so the oracle injects it the same way the Rust context does (explicitly).
  valid = File.join(tmp, 'valid')
  write(File.join(valid, 'tools/demo/index.yaml'), "name: demo\n")
  invalid = File.join(tmp, 'invalid')
  FileUtils.mkdir_p(invalid)
  home_valid = File.join(tmp, 'home-valid')
  write(File.join(home_valid, '.ukiryu/register/tools/demo/index.yaml'), "name: demo\n")
  home_invalid = File.join(tmp, 'home-invalid')
  FileUtils.mkdir_p(File.join(home_invalid, '.ukiryu/register'))
  home_empty = File.join(tmp, 'home-empty')
  FileUtils.mkdir_p(home_empty)
  dev_file = File.join(tmp, 'dev-file')
  write(dev_file, 'a file, not a directory')
  paths = { 'valid' => valid, 'invalid' => invalid, 'missing' => File.join(tmp, 'missing'), 'file' => dev_file,
            'none' => nil, 'empty_string' => '' }
  homes = { 'valid' => home_valid, 'invalid' => home_invalid, 'empty' => home_empty }
  saved = ENV.to_h.slice('UKIRYU_REGISTER', 'HOME', 'USERPROFILE')
  result['discovery'] = []
  begin
    %w[none valid invalid missing file empty_string].each do |env_key|
      %w[none valid invalid missing file].each do |dev_key|
        %w[valid invalid empty].each do |home_key|
          env_value = paths.fetch(env_key)
          env_value.nil? ? ENV.delete('UKIRYU_REGISTER') : ENV['UKIRYU_REGISTER'] = env_value
          ENV['HOME'] = homes.fetch(home_key)
          ENV['USERPROFILE'] = homes.fetch(home_key)
          dev = paths.fetch(dev_key)
          Register.define_singleton_method(:calculate_dev_path) { dev && Pathname.new(dev) }
          path, source = Register.send(:discover_path)
          result['discovery'] << {
            'env' => env_key, 'dev' => dev_key, 'home' => home_key,
            'discover_path' => [rel(path), source.to_s],
            'resolve_path_without_clone' => rel(Register.send(:resolve_path_without_clone)),
            'exists?' => typed(Register.exists?)
          }
        end
      end
    end
  ensure
    %w[UKIRYU_REGISTER HOME USERPROFILE].each { |k| saved.key?(k) ? ENV[k] = saved[k] : ENV.delete(k) }
  end

  # Clone/update against task-local synthetic repositories only.
  upstream = File.join(tmp, 'upstream')
  write(File.join(upstream, 'tools/demo/index.yaml'), "name: demo\n")
  git('init', '-q', chdir: upstream)
  git('add', '.', chdir: upstream)
  git('commit', '-q', '-m', 'initial', chdir: upstream)
  empty_upstream = File.join(tmp, 'empty-upstream')
  write(File.join(empty_upstream, 'README'), "no tools\n")
  git('init', '-q', chdir: empty_upstream)
  git('add', '.', chdir: empty_upstream)
  git('commit', '-q', '-m', 'initial', chdir: empty_upstream)
  original_remote = Register::REMOTE_URL
  set_remote = lambda do |url|
    Register.send(:remove_const, :REMOTE_URL)
    Register.const_set(:REMOTE_URL, url)
  end
  clone = {}
  begin
    set_remote.call(upstream)
    $tokens[upstream] = '<UPSTREAM>'
    fresh = Register.at(File.join(tmp, 'clones/fresh/register'))
    clone['fresh'] = outcome { fresh.clone!; nil }
    clone['fresh_after'] = { 'exists' => fresh.exists?, 'valid' => fresh.valid?, 'tool_names' => fresh.tool_names }
    clone['again'] = outcome { fresh.clone!; nil }
    clone['ensure_exists_valid'] = outcome { fresh.ensure_exists!; nil }
    git('mv', 'tools/demo', 'tools/demo2', chdir: upstream)
    git('commit', '-q', '-m', 'rename', chdir: upstream)
    clone['update'] = outcome { fresh.update!; nil }
    clone['update_after'] = fresh.tool_names
    via_update = Register.at(File.join(tmp, 'clones/via-update/register'))
    clone['update_missing_clones'] = outcome { via_update.update!; via_update.tool_names }
    not_git = File.join(tmp, 'clones/not-git')
    write(File.join(not_git, 'tools/demo/index.yaml'), "name: demo\n")
    r = Register.at(not_git)
    clone['update_not_git'] = outcome { r.update!; nil }
    clone['update_not_git_still_exists'] = File.exist?(File.join(not_git, 'tools/demo/index.yaml'))
    # Hazard: an existing invalid directory with content is removed by the source on clone failure.
    occupied = File.join(tmp, 'clones/occupied')
    write(File.join(occupied, 'user-work.txt'), "precious\n")
    r = Register.at(occupied)
    clone['ensure_exists_occupied'] = outcome { r.ensure_exists!; nil }
    clone['occupied_survives'] = File.exist?(File.join(occupied, 'user-work.txt'))
    set_remote.call(File.join(tmp, 'no-such-upstream'))
    bad = Register.at(File.join(tmp, 'clones/bad/register'))
    clone['bad_remote'] = outcome { bad.clone!; nil }
    clone['bad_remote_path_exists'] = File.exist?(bad.path)
    clone['bad_remote_parent_exists'] = File.exist?(File.dirname(bad.path))
    set_remote.call(empty_upstream)
    $tokens[empty_upstream] = '<EMPTY_UPSTREAM>'
    no_tools = Register.at(File.join(tmp, 'clones/no-tools/register'))
    clone['no_tools'] = outcome { no_tools.clone!; nil }
    clone['no_tools_path_exists'] = File.exist?(no_tools.path)
  ensure
    set_remote.call(original_remote)
  end
  # The pinned git gem raises Git::FailedError whose message embeds the full
  # command line and git's own stderr; only the Ukiryu-owned framing is stable.
  clone.each_value do |v|
    next unless v.is_a?(Hash) && v['error']

    m = v['error']['message']
    v['error']['first_line'] = m.lines.first.to_s.chomp.split(': [{', 2).first
    v['error']['ukiryu_tail'] = m.lines.drop_while { |l| !l.start_with?('To fix this:') && !l.start_with?('This error usually') }.join
    v['error'].delete('message') if m.lines.size > 1
  end
  result['clone'] = clone
  result['git_gem_version'] = Git::VERSION
end

puts JSON.pretty_generate(result)
