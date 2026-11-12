# frozen_string_literal: true
# Regenerate (Ruby 4.0.1, bundle of the pinned checkout's Gemfile.lock):
#   UKIRYU_RUBY_SRC=<ukiryu checkout at ab6df353> bundle exec ruby -I$UKIRYU_RUBY_SRC/lib capture_flow.rb > flow.json
# Oracle capture: version-detection strategy flow of the pinned source
# (ab6df353) with the process/man-page/platform boundaries replaced by
# deterministic stubs. The ORIGINAL methods run unchanged:
#   Ukiryu::VersionDetector.detect_info / detect_with_methods
#   Ukiryu::Tool::VersionDetection#detect_version (+ _with_detection_methods)
#   Ukiryu::Tool::Loader.run_detection_command / try_execute_command /
#     extract_version_from_pattern and the try_implementation presence check
# Stubbed: Executor.execute (scripted responses, argv recorded), Shell.detect,
# Platform.detect, ManPageParser.parse_date (owned by K21-MAN).
require 'json'
require 'ukiryu'
require 'ukiryu/execution'

# Harness failures must not be swallowed by the source's `rescue StandardError`.
class StubFailure < Exception; end

# Checkout of ukiryu/ukiryu at ab6df353c6dc2d7faf7817a86aafcafa82579896.
SRC = ENV.fetch('UKIRYU_RUBY_SRC')

module Stub
  class << self
    attr_accessor :calls, :responses, :platform, :man_dates, :man_calls, :shell_detect_calls, :platform_detect_calls

    def reset(responses:, platform: :linux, man_dates: {})
      @calls = []
      @responses = responses.dup
      @platform = platform
      @man_dates = man_dates
      @man_calls = []
      @shell_detect_calls = 0
      @platform_detect_calls = 0
    end
  end
end

def bytes_of(h, key)
  return '' unless h.key?(key)
  [h[key]].pack('H*').force_encoding('UTF-8')
end

Ukiryu::Executor.singleton_class.prepend(Module.new do
  def execute(executable, args = [], options = {})
    Stub.calls << { 'executable' => executable, 'args' => args, 'shell' => options[:shell]&.to_s,
                    'timeout' => options[:timeout], 'allow_failure' => options[:allow_failure] }
    resp = Stub.responses.shift
    raise StubFailure, 'stub: no scripted response' if resp.nil?
    raise StandardError, resp['raise'] if resp['raise'] # scripted runner failure

    begin
      out = Ukiryu::Execution::Output.new(stdout: bytes_of(resp, 'stdout_hex'), stderr: bytes_of(resp, 'stderr_hex'),
                                          exit_status: resp['status'])
      Ukiryu::Execution::Result.new(command_info: nil, output: out, metadata: nil)
    rescue StandardError => e
      raise StubFailure, "stub construction failed: #{e.class}: #{e.message}"
    end
  end
end)
Ukiryu::Shell.singleton_class.prepend(Module.new do
  def detect
    Stub.shell_detect_calls += 1
    :detected_shell
  end
end)
Ukiryu::Platform.singleton_class.prepend(Module.new do
  def detect
    Stub.platform_detect_calls += 1
    Stub.platform
  end
end)
Ukiryu::ManPageParser.singleton_class.prepend(Module.new do
  def parse_date(path)
    Stub.man_calls << path
    Stub.man_dates[path]
  end
end)

def hex(s) = s.b.unpack1('H*')
def resp(stdout: '', stderr: '', status: 0) = { 'stdout_hex' => hex(stdout), 'stderr_hex' => hex(stderr), 'status' => status }
def raise_resp(msg) = { 'raise' => msg }

def info_json(info)
  return nil if info.nil?
  { 'value' => info.value, 'method_used' => info.method_used.to_s,
    'available_methods' => info.available_methods.map(&:to_s), 'to_s' => info.to_s }
end

CASES = []

def record(id, api, input, responses, platform: :linux, man_dates: {})
  Stub.reset(responses: responses, platform: platform, man_dates: man_dates)
  error = nil
  result = begin
    yield
  rescue StandardError => e
    error = { 'class' => e.class.name, 'message' => e.message }
    nil
  end
  CASES << {
    'id' => id, 'api' => api, 'input' => input, 'responses' => responses,
    'platform' => platform.to_s, 'man_dates' => man_dates,
    'expected' => {
      'result' => result, 'raised' => error, 'calls' => Stub.calls,
      'unused_responses' => Stub.responses.size,
      'man_calls' => Stub.man_calls, 'shell_detect_calls' => Stub.shell_detect_calls,
      'platform_detect_calls' => Stub.platform_detect_calls
    }
  }
end

# Pattern given to detect_info: {'regexp_literal'=>'default'} means the source's /(\d+\.\d+)/ literal.
def pattern_value(p)
  p.is_a?(Hash) ? /(\d+\.\d+)/ : p
end

def di(id, executable: '/usr/bin/tool', command: '--version', pattern: { 'regexp_literal' => 'default' },
       shell: 'bash', source: 'command', timeout: 30, responses: [])
  input = { 'executable' => executable, 'command' => command, 'pattern' => pattern, 'shell' => shell,
            'source' => source, 'timeout' => timeout }
  record(id, 'detect_info', input, responses) do
    info_json(Ukiryu::VersionDetector.detect_info(executable: executable, command: command,
                                                  pattern: pattern_value(pattern), shell: shell&.to_sym,
                                                  source: source, timeout: timeout))
  end
end

# ---------- A. detect_info ----------
di('di/stdout-match', responses: [resp(stdout: "tool 1.2.3\n")])
di('di/stderr-only-match', responses: [resp(stdout: "usage\n", stderr: "tool 4.5\n")])
di('di/stdout-precedence', responses: [resp(stdout: "a 1.1", stderr: "b 2.2")])
di('di/no-match', responses: [resp(stdout: 'none', stderr: 'nothing')])
di('di/nonzero-exit-with-match', responses: [resp(stdout: '1.2', status: 1)])
di('di/exit-2', responses: [resp(stdout: '1.2', status: 2)])
di('di/runner-raises', responses: [raise_resp('boom')])
di('di/executable-nil', executable: nil, responses: [])
di('di/executable-empty', executable: '', responses: [])
di('di/command-array', command: ['-c', '1', 'localhost'], pattern: 'PING', responses: [resp(stdout: 'PING localhost')])
di('di/command-empty-array', command: [], responses: [resp(stdout: '3.4')])
di('di/shell-nil-detects', shell: nil, responses: [resp(stdout: '3.4')])
di('di/timeout-passthrough', timeout: 7, responses: [resp(stdout: '3.4')])
di('di/string-pattern', pattern: 'v(\d+)', responses: [resp(stdout: 'v12')])
di('di/no-capture-group', pattern: 'PING', responses: [resp(stdout: 'PING')])
di('di/optional-group-unmatched', pattern: '(x)?tool', responses: [resp(stdout: 'tool')])
di('di/ruby-invalid-pattern', pattern: '(', responses: [resp(stdout: '(1.2')])
di('di/ruby-invalid-pattern-man', pattern: '[a', source: 'man', executable: 'man', command: ['tool'], responses: [resp(stdout: 'x')])
di('di/invalid-utf8-scrubbed', pattern: "a(.)b", responses: [{ 'stdout_hex' => '61ff62', 'stderr_hex' => '', 'status' => 0 }])
di('di/invalid-utf8-truncated', pattern: "a(.+)b", responses: [{ 'stdout_hex' => '61e381ff62', 'stderr_hex' => '', 'status' => 0 }])
di('di/invalid-utf8-stderr', pattern: "v([^ ]+) ", responses: [{ 'stdout_hex' => '', 'stderr_hex' => '76c0af312e3220', 'status' => 0 }])
di('di/invalid-utf8-surrogate', pattern: "v(.+)$", responses: [{ 'stdout_hex' => '76eda080', 'stderr_hex' => '', 'status' => 0 }])
di('di/invalid-utf8-overlong4', pattern: "v(.+)$", responses: [{ 'stdout_hex' => '76f08080802e', 'stderr_hex' => '', 'status' => 0 }])
di('di/invalid-utf8-f5', pattern: "v(.+)$", responses: [{ 'stdout_hex' => '76f5808080', 'stderr_hex' => '', 'status' => 0 }])
di('di/invalid-utf8-trunc4', pattern: "v(.+)$", responses: [{ 'stdout_hex' => '76f09f98', 'stderr_hex' => '', 'status' => 0 }])
di('di/nul-bytes', pattern: '(\d+\.\d+)', responses: [resp(stdout: "\u0000 1.5 \u0000")])
di('di/multiline-anchor', pattern: '^tool (\d+\.\d+)$', responses: [resp(stdout: "banner\ntool 2.7\nmore")])
di('di/crlf', pattern: '(\d+\.\d+)$', responses: [resp(stdout: "tool 2.7\r\n")])
di('di/unicode-digits', pattern: '(\d+\.\d+)', responses: [resp(stdout: "v١.٢ then 8.9")])
# man source (legacy): tail of last 500 chars of stdout+stderr
long_head = "tool 9.9\n" + ('x' * 600)
di('man/match-in-tail', source: 'man', executable: 'man', command: ['find'], pattern: 'macOS ([\d.]+)',
   responses: [resp(stdout: ('y' * 700) + "\nmacOS 14.2  June 1, 2023")])
di('man/head-only-falls-back-to-command', source: 'man', executable: 'man', command: ['find'], pattern: 'tool ([\d.]+)',
   responses: [resp(stdout: long_head)])
di('man/short-output', source: 'man', executable: 'man', command: ['find'], pattern: 'macOS ([\d.]+)',
   responses: [resp(stdout: 'macOS 13.0')])
di('man/join-across-streams', source: 'man', executable: 'man', command: ['find'], pattern: 'macOS (\d+\.\d+)',
   responses: [resp(stdout: 'macOS 1', stderr: '2.5')])
di('man/join-not-in-command-fallback', source: 'man', executable: 'man', command: ['find'], pattern: 'X(\d)Y',
   responses: [resp(stdout: ('z' * 600) + 'X1', stderr: 'Y')])
di('man/multibyte-boundary', source: 'man', executable: 'man', command: ['find'], pattern: 'v(\d)',
   responses: [resp(stdout: "v1" + ("é" * 499) + 'end')])
di('man/multibyte-boundary-in', source: 'man', executable: 'man', command: ['find'], pattern: 'v(\d)',
   responses: [resp(stdout: "v1" + ("é" * 497))])
di('man/exactly-500', source: 'man', executable: 'man', command: ['find'], pattern: '^(\w)',
   responses: [resp(stdout: 'Q' + ('a' * 499))])
di('man/501', source: 'man', executable: 'man', command: ['find'], pattern: '\A(\w)',
   responses: [resp(stdout: 'Q' + ('a' * 500))])
di('man/stderr-only-in-tail', source: 'man', executable: 'man', command: ['find'], pattern: 'macOS ([\d.]+)',
   responses: [resp(stdout: 'a' * 800, stderr: 'macOS 12.1')])
di('man/nonzero', source: 'man', executable: 'man', command: ['find'], pattern: 'macOS ([\d.]+)',
   responses: [resp(stdout: 'macOS 12.1', status: 16)])
di('man/no-match', source: 'man', executable: 'man', command: ['find'], pattern: 'macOS ([\d.]+)',
   responses: [resp(stdout: 'nothing')])
di('source/other-value', source: 'weird', responses: [resp(stdout: '5.6')])

# ---------- B. detect_with_methods ----------
def dwm(id, methods:, executable: '/usr/bin/tool', shell: 'bash', timeout: 30, responses: [], platform: :linux, man_dates: {})
  input = { 'executable' => executable, 'methods' => methods, 'shell' => shell, 'timeout' => timeout }
  ruby_methods = methods.map do |m|
    h = { type: m['type'].to_sym }
    h[:command] = m['command'] if m.key?('command')
    h[:pattern] = pattern_value(m['pattern']) if m.key?('pattern')
    h[:paths] = m['paths'] if m.key?('paths')
    h
  end
  record(id, 'detect_with_methods', input, responses, platform: platform, man_dates: man_dates) do
    info_json(Ukiryu::VersionDetector.detect_with_methods(executable: executable, methods: ruby_methods,
                                                          shell: shell&.to_sym, timeout: timeout))
  end
end
MAN = '/usr/share/man/man1/tool.1'
cmd = { 'type' => 'command', 'command' => '--version', 'pattern' => 'tool ([\d.]+)' }
man = { 'type' => 'man_page', 'paths' => { 'macos' => MAN, 'linux' => MAN } }
dwm('dwm/command-first-wins', methods: [cmd, man], responses: [resp(stdout: 'tool 1.4')], man_dates: { MAN => '2020-09-21' })
dwm('dwm/command-fails-man-wins', methods: [cmd, man], responses: [resp(stdout: 'nope')], man_dates: { MAN => '2020-09-21' })
dwm('dwm/command-nonzero-man-wins', methods: [cmd, man], responses: [resp(stdout: 'tool 1.4', status: 1)], man_dates: { MAN => '2020-09-21' })
dwm('dwm/command-raises-man-wins', methods: [cmd, man], responses: [raise_resp('x')], man_dates: { MAN => '2020-09-21' })
dwm('dwm/man-first', methods: [man, cmd], responses: [], man_dates: { MAN => '2021-01-02' })
dwm('dwm/man-date-nil-then-command', methods: [man, cmd], responses: [resp(stdout: 'tool 2.0')], man_dates: {})
dwm('dwm/man-no-platform-path', methods: [{ 'type' => 'man_page', 'paths' => { 'macos' => MAN } }, cmd],
    responses: [resp(stdout: 'tool 2.0')], platform: :linux, man_dates: { MAN => '2020-01-01' })
dwm('dwm/man-windows', methods: [man], responses: [], platform: :windows, man_dates: { MAN => '2020-01-01' })
dwm('dwm/man-paths-missing', methods: [{ 'type' => 'man_page' }], responses: [])
dwm('dwm/all-fail', methods: [cmd, man], responses: [resp(stdout: 'x')], man_dates: {})
dwm('dwm/two-commands', methods: [cmd, { 'type' => 'command', 'command' => '-v', 'pattern' => 'v(\d+)' }],
    responses: [resp(stdout: 'x'), resp(stdout: 'v7')])
dwm('dwm/command-defaults', methods: [{ 'type' => 'command' }], responses: [resp(stdout: 'release 3.14.15')])
dwm('dwm/command-nil-values', methods: [{ 'type' => 'command', 'command' => nil, 'pattern' => nil }], responses: [resp(stdout: '3.1')])
dwm('dwm/unknown-type-skipped', methods: [{ 'type' => 'system_tool' }, cmd], responses: [resp(stdout: 'tool 5.0')])
dwm('dwm/available-uniq', methods: [cmd, man, cmd, man], responses: [resp(stdout: 'x'), resp(stdout: 'x')], man_dates: { MAN => '2020-02-02' })
dwm('dwm/empty-methods', methods: [], responses: [])
dwm('dwm/shell-nil', methods: [cmd], shell: nil, responses: [resp(stdout: 'tool 1.0')])
dwm('dwm/executable-nil', methods: [cmd, man], executable: nil, responses: [], man_dates: { MAN => '2020-02-02' })
dwm('dwm/invalid-pattern-next', methods: [{ 'type' => 'command', 'pattern' => '(' }, cmd],
    responses: [resp(stdout: 'tool 1.0'), resp(stdout: 'tool 1.0')])
dwm('dwm/no-group-value-nil', methods: [{ 'type' => 'command', 'pattern' => 'tool' }, cmd], responses: [resp(stdout: 'tool 1.0')])

# ---------- C. Tool::VersionDetection#detect_version (real model) ----------
class Probe
  include Ukiryu::Tool::VersionDetection
  def initialize(profile, executable, shell)
    @profile = profile
    @executable = executable
    @shell = shell
  end
end

def model_view(vd)
  return nil if vd.nil?
  {
    'command' => vd.command, 'pattern' => vd.pattern, 'source' => vd.source,
    'modern_threshold' => vd.modern_threshold,
    'detection_methods' => vd.detection_methods&.map do |m|
      { 'type' => m.type, 'command' => m.command, 'pattern' => m.pattern, 'paths' => m.paths }
    end
  }
end

def tdv(id, profile, origin:, executable: '/usr/bin/tool', shell: 'bash', responses: [], platform: :linux, man_dates: {})
  vd = profile.version_detection
  input = { 'origin' => origin, 'profile_version' => profile.version, 'version_detection' => model_view(vd),
            'executable' => executable, 'shell' => shell }
  record(id, 'tool_detect_version', input, responses, platform: platform, man_dates: man_dates) do
    info_json(Probe.new(profile, executable, shell&.to_sym).detect_version)
  end
end

REAL_OUTPUT = {
  'ghostscript' => '10.02.1', 'imagemagick' => "Version: ImageMagick 6.9.11-60 Q16 x86_64 https://imagemagick.org",
  'ping_bsd' => 'PING localhost (127.0.0.1): 56 data bytes', 'unzip' => 'UnZip 6.00 of 20 April 2009, by Debian.',
  'sort' => 'sort (GNU coreutils) 9.4', 'jq' => 'jq-1.7.1', 'yq' => 'yq (https://github.com/mikefarah/yq/) version v4.40.5',
  'gzip' => "gzip 1.12\nCopyright", 'gzip_gnu' => "gzip 1.12\nCopyright", 'bzip2' => 'bzip2, a block-sorting file compressor.  Version 1.0.8, 13-Jul-2019.',
  'bzip2_gnu' => 'bzip2, a block-sorting file compressor.  Version 1.0.8, 13-Jul-2019.', 'tar' => 'tar (GNU tar) 1.35',
  'inkscape' => 'Inkscape 1.2.2 (b0a8486541, 2022-12-01)', 'ping_gnu' => 'ping from iputils 20240117',
  'ping_windows' => 'Pinging 127.0.0.1 with 32 bytes of data:', 'exiftool' => '12.76', 'pdf2ps' => 'GPL Ghostscript 10.02.1',
  'ffmpeg' => 'ffmpeg version 6.1.1-3ubuntu5 Copyright (c) 2000-2023 the FFmpeg developers'
}.freeze
IM7 = "Version: ImageMagick 7.1.1-29 Q16-HDRI x86_64 22086 https://imagemagick.org"

fixture_files = Dir.glob("#{SRC}/spec/fixtures/{profiles,register/tools}/**/*.yaml").sort.reject { |f| f.end_with?('index.yaml') }
fixture_files.each do |path|
  rel = path.sub("#{SRC}/", '')
  text = File.read(path)
  next unless text.include?('version_detection')
  profile = Ukiryu::Models::ToolDefinition.from_yaml(text)
  out = REAL_OUTPUT.fetch(profile.name.to_s) { 'unknown 0.0' }
  out = IM7 if profile.name == 'imagemagick' && profile.version.to_s.start_with?('7')
  tdv("fixture/#{rel}/stdout", profile, origin: rel, responses: [resp(stdout: out)])
  tdv("fixture/#{rel}/stderr", profile, origin: rel, responses: [resp(stdout: "noise\n", stderr: out)])
  tdv("fixture/#{rel}/no-match", profile, origin: rel, responses: [resp(stdout: 'zzz', stderr: '')])
  tdv("fixture/#{rel}/nonzero", profile, origin: rel, responses: [resp(stdout: out, status: 1)])
  tdv("fixture/#{rel}/raises", profile, origin: rel, responses: [raise_resp('spawn failed')])
end

def synth(yaml_vd, version: '1.0')
  doc = { 'name' => 'synth', 'version' => version }
  doc['version_detection'] = yaml_vd unless yaml_vd == :absent
  Ukiryu::Models::ToolDefinition.from_yaml(doc.to_yaml)
end

tdv('synth/absent', synth(:absent), origin: 'synthetic')
tdv('synth/null', synth(nil), origin: 'synthetic')
tdv('synth/empty-command', synth({ 'command' => [], 'pattern' => '(\d+)' }), origin: 'synthetic')
tdv('synth/no-command', synth({ 'pattern' => '(\d+)' }), origin: 'synthetic')
tdv('synth/empty-pattern-profile', synth({ 'command' => '--version', 'pattern' => '' }, version: '3.3'), origin: 'synthetic')
tdv('synth/nil-pattern-profile', synth({ 'command' => '--version' }, version: '4.4'), origin: 'synthetic')
tdv('synth/man-source', synth({ 'command' => %w[man find], 'pattern' => 'macOS ([\d.]+)', 'source' => 'man' }),
    origin: 'synthetic', responses: [resp(stdout: ('m' * 700) + 'macOS 14.5')])
tdv('synth/man-source-only-man', synth({ 'command' => %w[man], 'pattern' => 'macOS ([\d.]+)', 'source' => 'man' }),
    origin: 'synthetic', responses: [resp(stdout: 'macOS 14.5')])
tdv('synth/source-null', synth({ 'command' => '--version', 'pattern' => '(\d+\.\d+)', 'source' => nil }),
    origin: 'synthetic', responses: [resp(stdout: '1.9')])
tdv('synth/executable-nil', synth({ 'command' => '--version', 'pattern' => '(\d+\.\d+)' }), origin: 'synthetic', executable: nil)
tdv('synth/shell-nil', synth({ 'command' => '--version', 'pattern' => '(\d+\.\d+)' }), origin: 'synthetic', shell: nil,
    responses: [resp(stdout: '1.9')])
tdv('synth/methods-command-then-man', synth({ 'detection_methods' => [
  { 'type' => 'command', 'command' => '-version', 'pattern' => 'Version: ImageMagick ([\d.]+)' },
  { 'type' => 'man_page', 'paths' => { 'macos' => '/usr/share/man/man1/convert.1', 'linux' => '/usr/share/man/man1/convert.1' } }
] }), origin: 'docs/advanced/writing-profiles.adoc', responses: [resp(stdout: 'no')], man_dates: { '/usr/share/man/man1/convert.1' => '2023-03-04' })
tdv('synth/methods-system-tool', synth({ 'detection_methods' => [
  { 'type' => 'command', 'command' => '--version', 'pattern' => 'Tool v([\d.]+)' },
  { 'type' => 'man_page', 'paths' => { 'macos' => '/usr/share/man/man1/tool.1', 'linux' => '/usr/share/man/man1/tool.1' } },
  { 'type' => 'system_tool' }
] }), origin: 'docs/advanced/writing-profiles.adoc', responses: [resp(stdout: 'no'), resp(stdout: 'release 2.5')], man_dates: {})
tdv('synth/methods-no-type', synth({ 'detection_methods' => [{ 'pattern' => 'v(\d+)' }] }), origin: 'synthetic',
    responses: [resp(stdout: 'v3')])
tdv('synth/methods-command-array', synth({ 'detection_methods' => [{ 'type' => 'command', 'command' => %w[-a -b], 'pattern' => 'v(\d+)' }] }),
    origin: 'synthetic', responses: [resp(stdout: 'v3')])
tdv('synth/methods-man-first-success', synth({ 'detection_methods' => [
  { 'type' => 'man_page', 'paths' => { 'linux' => '/m.1' } }, { 'type' => 'command' }
] }), origin: 'synthetic', responses: [], man_dates: { '/m.1' => '2019-05-06' })
tdv('synth/methods-empty-falls-to-legacy', synth({ 'detection_methods' => [], 'command' => '--version', 'pattern' => '(\d+\.\d+)' }),
    origin: 'synthetic', responses: [resp(stdout: '7.7')])
tdv('synth/methods-and-legacy', synth({ 'detection_methods' => [{ 'type' => 'command', 'pattern' => 'm(\d+)' }],
                                        'command' => '-x', 'pattern' => '(\d+\.\d+)' }),
    origin: 'synthetic', responses: [resp(stdout: 'm5 1.1')])
tdv('synth/methods-pattern-empty', synth({ 'detection_methods' => [{ 'type' => 'command', 'pattern' => '' }] }),
    origin: 'synthetic', responses: [resp(stdout: '1.1')])

# ---------- D. Loader implementation-index probe (live copy: Tool::Loader) ----------
L = Ukiryu::Tool::Loader
def ip(id, detection, tool_name: 'tool', options: {}, responses: [])
  opts = options.transform_keys(&:to_sym)
  opts[:shell] = opts[:shell].to_sym if opts[:shell]
  record(id, 'index_probe', { 'detection' => detection, 'tool_name' => tool_name, 'options' => options }, responses) do
    det = detection
    out = L.send(:run_detection_command, det, tool_name, opts)
    if out.nil?
      { 'output' => nil, 'outcome' => 'not_detected' }
    else
      pattern = det[:pattern] || det['pattern']
      version = L.send(:extract_version_from_pattern, out, pattern)
      skip = false
      if version.nil? && pattern
        skip = !out.match?(Regexp.new(pattern))
      end
      { 'output' => out, 'outcome' => skip ? 'not_detected' : 'detected', 'version' => version }
    end
  end
end
ip('ip/executable-key', { 'executable' => 'ping', 'command' => ['-c', '1', 'localhost'], 'pattern' => 'PING' },
   options: { 'shell' => 'bash' }, responses: [resp(stdout: 'PING localhost')])
ip('ip/tool-name-fallback', { 'command' => ['--version'], 'pattern' => 'sort ([\d.]+)' }, tool_name: 'sort',
   options: { 'shell' => 'bash' }, responses: [resp(stdout: 'sort (GNU coreutils) 9.4')])
ip('ip/options-executable', { 'command' => '--version', 'pattern' => 'sort \(GNU coreutils\) ([\d.]+)' }, tool_name: 'sort',
   options: { 'shell' => 'bash', 'executable' => '/opt/sort' }, responses: [resp(stdout: 'sort (GNU coreutils) 9.4')])
ip('ip/no-command', { 'pattern' => 'x' }, options: { 'shell' => 'bash' })
ip('ip/executables-first-fails', { 'executables' => %w[magick convert], 'command' => ['-version'], 'pattern' => 'ImageMagick ([\d.]+)' },
   options: { 'shell' => 'bash' }, responses: [raise_resp('ENOENT'), resp(stdout: 'Version: ImageMagick 6.9.11')])
ip('ip/executables-nonzero-empty-stderr', { 'executables' => %w[magick convert], 'command' => ['-version'], 'pattern' => 'ImageMagick ([\d.]+)' },
   options: { 'shell' => 'bash' }, responses: [resp(stdout: 'x', stderr: "  \n", status: 1), resp(stdout: 'Version: ImageMagick 7.1')])
ip('ip/executables-all-fail', { 'executables' => %w[a b], 'command' => ['-v'], 'pattern' => 'x' },
   options: { 'shell' => 'bash' }, responses: [resp(status: 1), resp(status: 127)])
ip('ip/nonzero-uses-stderr', { 'executable' => 'busybox', 'command' => ['gzip', '--help'], 'pattern' => 'BusyBox v([\d.]+)' },
   options: { 'shell' => 'sh' }, responses: [resp(stdout: 'ignored 9.9', stderr: 'BusyBox v1.36.1 (2023)', status: 1)])
ip('ip/presence-only', { 'executable' => 'ping', 'command' => ['-V'], 'pattern' => 'ping from iputils' },
   options: { 'shell' => 'bash' }, responses: [resp(stdout: 'ping from iputils 20240117')])
ip('ip/presence-miss', { 'executable' => 'ping', 'command' => ['-V'], 'pattern' => 'ping from iputils' },
   options: { 'shell' => 'bash' }, responses: [resp(stdout: 'PING ok')])
ip('ip/no-pattern', { 'executable' => 'x', 'command' => ['-v'] }, options: { 'shell' => 'bash' }, responses: [resp(stdout: 'anything')])
ip('ip/empty-success-output', { 'executable' => 'x', 'command' => ['-v'], 'pattern' => '(\d+)' }, options: { 'shell' => 'bash' },
   responses: [resp(stdout: '')])
ip('ip/scrub-removes', { 'executable' => 'x', 'command' => ['-v'], 'pattern' => 'a(.)b' }, options: { 'shell' => 'bash' },
   responses: [{ 'stdout_hex' => '61ff2d62', 'stderr_hex' => '', 'status' => 0 }])
ip('ip/stderr-invalid-utf8-strip', { 'executable' => 'x', 'command' => ['-v'], 'pattern' => 'v(\d)' }, options: { 'shell' => 'bash' },
   responses: [{ 'stdout_hex' => '', 'stderr_hex' => '2076ff3120', 'status' => 1 }])
ip('ip/stderr-only-invalid', { 'executable' => 'x', 'command' => ['-v'], 'pattern' => 'v(\d)' }, options: { 'shell' => 'bash' },
   responses: [{ 'stdout_hex' => '', 'stderr_hex' => 'ff', 'status' => 1 }])
ip('ip/shell-nil-detects', { 'executable' => 'x', 'command' => ['-v'], 'pattern' => '(\d)' }, options: {}, responses: [resp(stdout: '1')])
ip('ip/invalid-pattern', { 'executable' => 'x', 'command' => ['-v'], 'pattern' => '(' }, options: { 'shell' => 'bash' },
   responses: [resp(stdout: '1')])
ip('ip/symbol-keys-ignored-string', { 'executable' => 'x', 'command' => '-v', 'pattern' => 'v(\d)' }, options: { 'shell' => 'bash' },
   responses: [resp(stdout: 'v2')])

[["nul", "\u0000"], ["ws", " \t\n\v\f\r"], ["nbsp", "\u00a0"], ["nul-x", "\u0000x"], ["x-nul", "x\u0000"],
 ["nul-mid-ws", " \u0000 "], ["ws-nul-ws", "\u0000 \u0000"], ["empty", ""]].each do |name, err|
  ip("ip/failure-stderr-#{name}", { 'executable' => 'x', 'command' => ['-v'] }, options: { 'shell' => 'bash' },
     responses: [resp(stdout: 'out', stderr: err, status: 3)])
end

puts JSON.pretty_generate({
  'provenance' => {
    'oracle' => 'ukiryu/ukiryu ab6df353c6dc2d7faf7817a86aafcafa82579896', 'ruby' => RUBY_VERSION,
    'script' => 'capture_flow.rb',
    'sources' => %w[lib/ukiryu/version_detector.rb lib/ukiryu/tool/version_detection.rb lib/ukiryu/tool/loader.rb],
    'stubs' => 'Executor.execute, Shell.detect, Platform.detect, ManPageParser.parse_date'
  },
  'cases' => CASES
})
