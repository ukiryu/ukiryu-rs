# frozen_string_literal: true

# Capture the Tool::Loader reference from the pinned Ruby source.
#
#   ruby fixtures/tool_loader/capture-loader.rb <ruby-reference> > fixtures/tool_loader/reference.json
#
# Three groups of observations, all from the unmodified source at REVISION:
#
# * transforms: the loader's private Hash helpers (deep_merge_hashes,
#   convert_actions_to_array, build_interface_commands_hash,
#   flatten_signature, extract_profile_data) with the caller's arguments
#   before and after the call, so in-place mutation is recorded too.
# * conversions: convert_to_tool_definition over every version file of the
#   register fixture (fixtures/register/spec, a byte copy of the pinned
#   spec/fixtures/register) for each distinct profile choice, with and
#   without a detected version.
# * loads: load_with_implementation_index end to end, over the register
#   fixture and over small synthetic registers this script writes into a
#   scratch directory (their files are recorded so the test can rebuild
#   them). Three process-facing collaborators are replaced and recorded:
#   Platform.detect returns the row's current platform, Executor.execute
#   answers from the row's scripted table (anything else raises ENOENT, which
#   the loader rescues), and Tool.new returns the definition it was given, so
#   the row records the ToolDefinition the loader built. The shell is always
#   passed explicitly; Shell.detect is never reached.
#
# Values are encoded with $sym / $float / $map tags (the models' oracle codec);
# every Hash is a $map so key order is kept.
require 'json'
require 'tmpdir'
require 'fileutils'
require 'digest'

REVISION = 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
root = File.expand_path(ARGV.fetch(0))
revision = IO.popen(['git', '-C', root, 'rev-parse', 'HEAD'], &:read).strip
raise "wrong reference revision #{revision}" unless revision == REVISION

$LOAD_PATH.unshift(File.join(root, 'lib'))
require 'ukiryu'
require 'ukiryu/tool/loader'
require 'yaml'
%w[implementation_version implementation_index interface tool_definition platform_profile
   command_definition].each { |f| require "ukiryu/models/#{f}" }

FIXTURE = File.expand_path('../register/spec', __dir__)
SOURCE_FIXTURE = File.join(root, 'spec', 'fixtures', 'register')
Loader = Ukiryu::Tool::Loader
M = Ukiryu::Models

def enc(v)
  case v
  when nil, true, false, String, Integer then v
  when Float then { '$float' => v.to_s }
  when Symbol then { '$sym' => v.to_s }
  when Array then v.map { |x| enc(x) }
  when Hash then { '$map' => v.map { |k, x| [enc(k), enc(x)] } }
  else raise "unencodable #{v.class}"
  end
end

def err(e)
  { 'error' => { 'class' => e.class.name, 'message' => e.message.lines.first.to_s.chomp } }
end

# ---------------------------------------------------------------- stubs
$current_platform = nil
$table = {}
$calls = nil
$specs = nil

Ukiryu::Platform.singleton_class.prepend(Module.new do
  def detect
    $current_platform || super
  end
end)

Ukiryu::Executor.singleton_class.prepend(Module.new do
  def execute(executable, args = [], options = {})
    return super unless $calls

    $calls << { 'executable' => executable, 'args' => args, 'shell' => options[:shell]&.to_s,
                'timeout' => options[:timeout], 'allow_failure' => options[:allow_failure] }
    answer = $table[([executable] + args).join(' ')]
    raise Errno::ENOENT, executable unless answer

    Struct.new(:status, :stdout, :stderr) do
      def success? = status.zero?
    end.new(answer['status'], answer['stdout'], answer['stderr'])
  end
end)

Ukiryu::Tool.singleton_class.prepend(Module.new do
  def new(definition, options = {})
    return super unless $calls

    [:built, definition]
  end
end)

Loader.singleton_class.prepend(Module.new do
  def detect_implementation_and_version(index, tool_name, options = {})
    spec = super
    $specs&.<<(spec && spec.to_h { |k, v| [k.to_s, v.nil? ? nil : v.to_s] })
    spec
  end
end)

$definitions = {}
def definition_ref(td)
  hash = enc(td.to_hash)
  key = Digest::SHA256.hexdigest(JSON.generate(hash))[0, 16]
  $definitions[key] = hash
  key
end

# ----------------------------------------------------------- transforms
cases = []
merges = [
  [{}, {}], [{ a: 1 }, { a: nil }], [{ a: nil }, { a: 2 }],
  [{ a: { b: 1, c: 2 } }, { a: { b: nil, c: 3, d: 4 } }],
  [{ a: [1, 2] }, { a: [3] }], [{ 'a' => 1 }, { a: 2 }],
  [{ 1 => 'integer', 1.0 => 'float' }, { 1 => nil }],
  [{ a: false }, { a: 0 }], [nil, {}], [{}, nil],
  [{ a: 1 }, 3], [[1], {}], [{ b: 1, a: 2 }, { c: 3, a: { x: 1 } }]
]
merges.each_with_index { |args, i| cases << ['deep_merge_hashes', "merge-#{i}", args] }
actions = [nil, {}, [], { 'run' => {} }, { run: { description: 'Run' } },
           { 'run' => { 'name' => 'explicit' } }, { 'run' => { 'name' => false } },
           { 'run' => nil }, { 'run' => false }, [{ name: 'run' }],
           [{ name: 'run', signature: { inputs: { inputs: [{ name: 'file' }], options: [{ name: 'level' }] } } }],
           [{ 'name' => 'run', 'signature' => { 'flags' => [{ 'name' => 'verbose' }] } }],
           [{ name: 'one' }, { name: 'one' }], false, 3,
           '', 'x', [nil], [{}], { 1 => {} }, [3]]
actions.each_with_index do |value, i|
  cases << ['convert_actions_to_array', "actions-#{i}", [value]]
  cases << ['build_interface_commands_hash', "interface-#{i}", [value]]
end
signatures = [{}, { signature: nil }, { signature: false },
              { signature: { inputs: { inputs: [{ name: 'file' }], flags: [{ name: 'verbose' }] } } },
              { 'signature' => { 'inputs' => { 'inputs' => [], 'signature' => 'ignored' } } },
              { name: 'old', signature: { name: 'new', signature: 'ignored' } },
              { signature: {}, 'signature' => { name: 'string-key' } },
              { signature: 'invalid' }, { signature: 3 },
              { signature: { inputs: [1, 2], outputs: { inputs: 1 } } },
              { signature: { inputs: { 1 => 'one', 'inputs' => 'args' } } }]
signatures.each_with_index { |value, i| cases << ['flatten_signature', "signature-#{i}", [value]] }
profiles = [{}, { name: 'symbol', 'name' => 'string' }, { name: false, 'name' => 'string' },
            { 'platforms' => ['linux'], 'shells' => ['bash'], 'option_style' => 'gnu' }, nil, false,
            { inherits: 'base', executable_name: 'x', commands: [] }]
profiles.each_with_index { |value, i| cases << ['extract_profile_data', "profile-#{i}", [value]] }
transforms = cases.map do |method, id, original|
  args = Marshal.load(Marshal.dump(original))
  before = enc(args)
  result = begin
    { 'ok' => enc(Loader.send(method, *args)) }
  rescue StandardError => e
    err(e)
  end
  { 'id' => id, 'method' => method, 'before' => before, 'after' => enc(args), 'result' => result }
end

# ---------------------------------------------------------- conversions
PLATFORMS = %i[linux macos windows].freeze
SHELLS = %i[bash zsh sh fish powershell cmd].freeze
def register_hash(text) = Ukiryu::Utils.symbolize_keys(YAML.safe_load(text, permitted_classes: [Symbol], aliases: true))

conversions = []
Dir.glob("#{FIXTURE}/tools/*/index.yaml").sort.each do |idx_file|
  tool = File.basename(File.dirname(idx_file))
  index = M::ImplementationIndex.from_hash(register_hash(File.read(idx_file)))
  iface = M::Interface.from_hash(register_hash(File.read(File.join(FIXTURE, 'interfaces', "#{index.interface}.yaml"))))
  index.implementations.each do |impl|
    impl_name = impl[:name].to_s
    (impl[:versions] || []).each do |vs|
      path = File.join(FIXTURE, 'tools', tool, impl_name, vs[:file])
      next unless File.exist?(path)

      ver = M::ImplementationVersion.from_hash(register_hash(File.read(path)))
      seen = {}
      PLATFORMS.product(SHELLS).each do |plat, sh|
        prof = ver.compatible_profile(platform: plat, shell: sh)
        key = prof ? (prof[:name] || prof['name']).to_s : '<none>'
        next if seen[key]

        seen[key] = true
        [nil, '9.9'].each do |detected|
          out = begin
            td = Loader.send(:convert_to_tool_definition, tool, iface, ver, impl_name, detected,
                             platform: plat, shell: sh)
            { 'ok' => td && definition_ref(td) }
          rescue StandardError => e
            err(e)
          end
          conversions << { 'tool' => tool, 'implementation' => impl_name, 'file' => vs[:file],
                           'platform' => plat.to_s, 'shell' => sh.to_s, 'detected' => detected,
                           'result' => out }
        end
      end
    end
  end
end

# ---------------------------------------------------------------- loads
def ok(stdout) = { 'status' => 0, 'stdout' => stdout, 'stderr' => '' }
TABLES = {
  'none' => {},
  'present' => {
    'gzip --version' => ok("gzip 1.0\n"),
    'gzip --help' => { 'status' => 1, 'stdout' => '', 'stderr' => "BusyBox v1.36.1 multi-call binary.\n" },
    'bzip2 --help' => { 'status' => 0, 'stdout' => '', 'stderr' => "bzip2 1.0.8\n" },
    'convert -version' => ok("Version: ImageMagick 6.9.12-98 Q16 x86_64\n"),
    'ping -V' => { 'status' => 2, 'stdout' => '', 'stderr' => "  \n" },
    'ping -c 1 localhost' => ok("PING localhost (127.0.0.1): 56 data bytes\n"),
    'tar --version' => ok("tar (GNU tar) 1.35\n"),
    'ping.exe -n 1 127.0.0.1' => ok("Pinging 127.0.0.1 with 32 bytes of data:\n")
  },
  'modern' => {
    'magick -version' => ok("Version: ImageMagick 7.1.1-15 Q16-HDRI\n"),
    'gzip --version' => ok("gzip 1.12\n"),
    'gzip --help' => ok("Usage: gzip [OPTION]... [FILE]...\n"),
    'ping -V' => ok("ping from iputils 20240117\n"),
    'ping -c 1 localhost' => { 'status' => 68, 'stdout' => '', 'stderr' => "ping: cannot resolve localhost\n" },
    'tar --version' => ok("tar 1.0\n")
  }
}.freeze

REQUESTS = [
  %w[linux - bash], %w[macos - zsh], %w[windows - powershell], %w[windows - cmd],
  %w[linux linux bash], %w[linux windows cmd], %w[linux windows powershell],
  %w[windows macos zsh], %w[macos linux fish]
].freeze

def load_row(register, tool, current, platform, shell, table_name, table)
  $current_platform = current.to_sym
  $table = table
  $calls = []
  $specs = []
  options = { register_path: register, shell: shell.to_sym }
  options[:platform] = platform.to_sym unless platform == '-'
  result = begin
    built = Loader.load_with_implementation_index(tool, options)
    case built
    when nil then { 'ok' => nil }
    when Array then { 'ok' => definition_ref(built[1]) }
    else raise "unexpected #{built.class}"
    end
  rescue StandardError => e
    err(e)
  end
  row = { 'tool' => tool, 'current' => current, 'platform' => platform == '-' ? nil : platform,
          'shell' => shell, 'table' => table_name, 'calls' => $calls, 'spec' => $specs.first, 'result' => result }
  $calls = nil
  $specs = nil
  $current_platform = nil
  row
end

loads = []
tools = Dir.glob("#{FIXTURE}/tools/*/index.yaml").map { |f| File.basename(File.dirname(f)) }.sort
(tools + ['missing_tool']).each do |tool|
  REQUESTS.each do |current, platform, shell|
    detecting = platform == '-' || platform == current
    (detecting ? TABLES.keys : ['none']).each do |name|
      loads << load_row(FIXTURE, tool, current, platform, shell, name, TABLES[name])
    end
  end
end

# Synthetic registers: one tool `t` each, files recorded verbatim.
BASE_INTERFACE = <<~YAML
  name: t
  version: '1.0'
  actions:
    run:
      description: Run it
      options:
        - name: level
          cli: "-l"
    pack:
      description: Pack
YAML
def version_doc(extra = '')
  <<~YAML
    implements: t
    version: '1.0'
    execution_profiles:
      - name: unix
        platforms: [linux, macos]
        shells: [bash, zsh]
        executable_name: t
        option_style: double_dash_equals
        commands:
          - name: run
            options:
              - name: level
                cli: "--level"
                type: integer
          - subcommand: pack
            description: Pack files
      - name: win
        platforms: [windows]
        shells: [powershell]
        inherits: unix
    #{extra}
  YAML
end
def index_doc(impls) = "name: t\ninterface: t\nimplementations:\n#{impls}"
SYNTHETIC = {
  'merge' => {
    'tools/t/index.yaml' => index_doc(<<~YAML),
      - name: default
        detection:
          command: ["--version"]
          pattern: 't ([\\d.]+)'
        version_scheme: semantic
        versions:
          - equals: '1.0'
            file: 1.0.yaml
    YAML
    'tools/t/default/1.0.yaml' => version_doc("aliases: [tee]\ndisplay_name: T Tool\n"),
    'interfaces/t.yaml' => BASE_INTERFACE
  },
  'interface_actions' => {
    'tools/t/index.yaml' => index_doc(<<~YAML),
      - name: alt
        detection:
          command: ["-v"]
        versions:
          - file: 2.0.yaml
            default: true
    YAML
    'tools/t/alt/2.0.yaml' => <<~YAML,
      implements: t
      version: 2.0
      execution_profiles:
        - name: any
          platforms: [linux, macos, windows]
          shells: [bash, zsh, powershell, cmd]
    YAML
    'interfaces/t.yaml' => <<~YAML
      name: t
      actions:
        - name: run
          signature:
            inputs:
              inputs:
                - name: file
              options:
                - name: level
                  cli: "-l"
            flags:
              - name: verbose
                cli: "-v"
        - name: pack
    YAML
  },
  'no_detection' => {
    'tools/t/index.yaml' => index_doc("  - name: default\n    versions:\n      - equals: '1.0'\n        file: 1.0.yaml\n"),
    'tools/t/default/1.0.yaml' => version_doc,
    'interfaces/t.yaml' => BASE_INTERFACE
  },
  'bad_scheme' => {
    'tools/t/index.yaml' => index_doc(<<~YAML),
      - name: default
        detection:
          command: ["--version"]
          pattern: 't ([\\d.]+)'
        version_scheme: nonexistent
        versions:
          - equals: '1.0'
            file: 1.0.yaml
    YAML
    'tools/t/default/1.0.yaml' => version_doc,
    'interfaces/t.yaml' => BASE_INTERFACE
  },
  'invalid_pattern' => {
    'tools/t/index.yaml' => index_doc(<<~YAML),
      - name: default
        detection:
          command: ["--version"]
          pattern: 't ([\\d.]+'
        versions:
          - equals: '1.0'
            file: 1.0.yaml
    YAML
    'tools/t/default/1.0.yaml' => version_doc,
    'interfaces/t.yaml' => BASE_INTERFACE
  },
  'missing_interface' => {
    'tools/t/index.yaml' => "name: t\ninterface: absent\nimplementations: []\n",
    'interfaces/t.yaml' => BASE_INTERFACE
  },
  'no_implementations' => {
    'tools/t/index.yaml' => "name: t\ninterface: t\nimplementations: []\n",
    'interfaces/t.yaml' => BASE_INTERFACE
  },
  'missing_version_file' => {
    'tools/t/index.yaml' => index_doc("  - name: default\n    detection:\n      command: [\"--version\"]\n    versions:\n      - equals: '1.0'\n        file: 1.0.yaml\n"),
    'interfaces/t.yaml' => BASE_INTERFACE
  },
  'empty_index' => {
    'tools/t/index.yaml' => "---\n",
    'interfaces/t.yaml' => BASE_INTERFACE
  },
  'nil_file' => {
    'tools/t/index.yaml' => index_doc("  - name: default\n    detection:\n      command: [\"--version\"]\n    versions:\n      - equals: '1.0'\n"),
    'interfaces/t.yaml' => BASE_INTERFACE
  },
  'calver' => {
    'tools/t/index.yaml' => index_doc(<<~YAML),
      - name: first
        detection:
          command: ["--first"]
          pattern: 'first'
        versions:
          - equals: '1.0'
            file: 1.0.yaml
      - name: dated
        detection:
          command: ["--version"]
          pattern: 'dated (\\d{4}\\.\\d{2}\\.\\d{2})'
        version_scheme: calver
        versions:
          - equals: '2024.01.17'
            file: 1.0.yaml
          - after: '2024.01.01'
            file: latest.yaml
        default: 1.0.yaml
    YAML
    'tools/t/first/1.0.yaml' => version_doc,
    'tools/t/dated/1.0.yaml' => version_doc,
    'tools/t/dated/latest.yaml' => version_doc("display_name: Latest\n").sub("version: '1.0'", 'version: latest'),
    'interfaces/t.yaml' => BASE_INTERFACE
  }
}.freeze
SYNTHETIC_TABLES = {
  'none' => {},
  'versioned' => { 't --version' => ok("t 1.0\n"), 't -v' => ok("t 2\n"),
                   't --first' => { 'status' => 1, 'stdout' => '', 'stderr' => '' } },
  'other' => { 't --version' => ok("t 9.9\n"), 't --first' => ok("not it\n") },
  'dated' => { 't --version' => ok("dated 2024.01.17\n") },
  'dated_later' => { 't --version' => ok("dated 2024.06.30\n") }
}.freeze
SYNTHETIC_REQUESTS = [%w[linux - bash], %w[windows - powershell], %w[linux windows powershell],
                      %w[macos - zsh], %w[windows - cmd]].freeze

synthetic = []
Dir.mktmpdir('ukiryu-loader-') do |scratch|
  SYNTHETIC.each do |name, files|
    dir = File.join(scratch, name)
    files.each do |rel, text|
      FileUtils.mkdir_p(File.dirname(File.join(dir, rel)))
      File.write(File.join(dir, rel), text)
    end
    rows = []
    SYNTHETIC_REQUESTS.each do |current, platform, shell|
      detecting = platform == '-' || platform == current
      (detecting ? SYNTHETIC_TABLES.keys : ['none']).each do |table|
        rows << load_row(dir, 't', current, platform, shell, table, SYNTHETIC_TABLES[table])
      end
    end
    synthetic << { 'name' => name, 'files' => files, 'loads' => rows }
  end
end

def table_json(tables) = tables.transform_values { |t| t }
fixture_files = Dir.glob('**/*', base: FIXTURE).reject { |p| File.directory?(File.join(FIXTURE, p)) }.sort
proof = fixture_files.all? { |p| File.binread(File.join(FIXTURE, p)) == File.binread(File.join(SOURCE_FIXTURE, p)) }
raise 'register fixture differs from the pinned source' unless proof

puts JSON.generate({
                            'source' => REVISION,
                            'loader_sha256' => Digest::SHA256.file(File.join(root, 'lib/ukiryu/tool/loader.rb')).hexdigest,
                            'ruby' => RUBY_VERSION,
                            'lutaml_model' => Gem.loaded_specs['lutaml-model']&.version.to_s,
                            'versionian' => Gem.loaded_specs['versionian']&.version.to_s,
                            'producer' => 'fixtures/tool_loader/capture-loader.rb',
                            'tables' => table_json(TABLES),
                            'synthetic_tables' => table_json(SYNTHETIC_TABLES),
                            'transforms' => transforms,
                            'conversions' => conversions,
                            'loads' => loads,
                            'synthetic' => synthetic,
                            'definitions' => $definitions
                          })
