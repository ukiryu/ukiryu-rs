# frozen_string_literal: true
# Oracle capture: Ukiryu::VersionDetector.detect_with_methods with the REAL
# Ukiryu::ManPageParser.parse_date (pinned ukiryu ab6df353), so man-page
# errors (Date::Error, RangeError, ArgumentError, Errno::EISDIR) are observed
# exactly as they escape detect_with_methods. Stubbed: Executor.execute
# (scripted responses, argv recorded), Shell.detect, Platform.detect (:linux).
# Man pages are written to a temp dir; paths are reported as "<man>/<name>".
# Regenerate: rb.sh capture_man_detection.rb > detect_with_methods.json (TZ=UTC).
require 'json'
require 'tmpdir'
require 'date'
require 'ukiryu'
require 'ukiryu/execution'

class StubFailure < Exception; end

module Stub
  class << self
    attr_accessor :calls, :responses
  end
end

Ukiryu::Executor.singleton_class.prepend(Module.new do
  def execute(executable, args = [], options = {})
    Stub.calls << { 'executable' => executable, 'args' => args, 'shell' => options[:shell]&.to_s,
                    'timeout' => options[:timeout], 'allow_failure' => options[:allow_failure] }
    resp = Stub.responses.shift
    raise StubFailure, 'stub: no scripted response' if resp.nil?

    out = Ukiryu::Execution::Output.new(stdout: [resp['stdout_hex']].pack('H*').force_encoding('UTF-8'),
                                        stderr: '', exit_status: resp['status'])
    Ukiryu::Execution::Result.new(command_info: nil, output: out, metadata: nil)
  end
end)
Ukiryu::Shell.singleton_class.prepend(Module.new { def detect = :detected_shell })
Ukiryu::Platform.singleton_class.prepend(Module.new { def detect = :linux })

def hex(s) = s.b.unpack1('H*')
def resp(stdout, status: 0) = { 'stdout_hex' => hex(stdout), 'status' => status }

DIR = Dir.mktmpdir('man-detect')
PAGES = {
  'full_date' => ".Dd September 21, 2020\n.Dt XARGS 1\n",
  'partial_date' => ".Dd September 21\n.Dt XARGS 1\n",
  'invalid_date' => ".Dd September 31, 2020\n",
  'range_error' => ".Dd Sep 99999999999, 2020\n",
  'invalid_utf8' => "\xFF\n.Dd May 1, 2001\n".b,
  'no_dd' => ".TH XARGS 1 \"2020-09-21\"\n"
}.freeze
PAGES.each { |name, body| File.binwrite(File.join(DIR, name), body) }
Dir.mkdir(File.join(DIR, 'a_directory'))

def real(name) = File.join(DIR, name)
def virt(name) = "<man>/#{name}"
def sanitize(s) = s.gsub(DIR, '<man>')

def man(name, platform: :linux) = { 'type' => 'man_page', 'paths' => { platform.to_s => name } }
def cmd(pattern = nil) = { 'type' => 'command', 'command' => ['--version'], 'pattern' => pattern }

CASES = []
def case_(id, methods, responses = [])
  Stub.calls = []
  Stub.responses = responses.dup
  ruby_methods = methods.map do |m|
    if m['type'] == 'man_page'
      { type: :man_page, paths: m['paths'].to_h { |k, v| [k.to_sym, real(v)] } }
    else
      { type: :command, command: m['command'], pattern: m['pattern'] ? Regexp.new(m['pattern']) : nil }.compact
    end
  end
  raised = nil
  result = begin
    info = Ukiryu::VersionDetector.detect_with_methods(executable: '/usr/bin/tool', methods: ruby_methods,
                                                        shell: 'bash', timeout: 30)
    info && { 'value' => info.value, 'method_used' => info.method_used.to_s,
              'available_methods' => info.available_methods.map(&:to_s) }
  rescue StandardError => e
    raised = { 'class' => e.class.name, 'message' => sanitize(e.message) }
    nil
  end
  CASES << {
    'id' => id,
    'methods' => methods.map { |m| m['type'] == 'man_page' ? m.merge('paths' => m['paths'].transform_values { virt(_1) }) : m.compact },
    'responses' => responses,
    'expected' => { 'result' => result, 'raised' => raised, 'calls' => Stub.calls,
                    'unused_responses' => Stub.responses.size }
  }
end

case_('ok/full-date', [man('full_date')])
case_('ok/partial-date-uses-today', [man('partial_date')])
case_('nil/no-dd', [man('no_dd')])
case_('nil/missing-file', [man('missing')])
case_('nil/other-platform-only', [man('invalid_date', platform: :macos)])
case_('err/date-error', [man('invalid_date')])
case_('err/range-error', [man('range_error')])
case_('err/invalid-utf8', [man('invalid_utf8')])
case_('err/directory', [man('a_directory')])
case_('err/stops-before-later-command', [man('invalid_date'), cmd], [resp('tool 1.4')])
case_('err/after-command-miss', [cmd, man('invalid_date')], [resp('nope')])
case_('err/first-man-error-stops-second-man', [man('range_error'), man('full_date')])
case_('ok/command-hit-skips-bad-man', [cmd, man('invalid_date')], [resp('tool 1.4')])
case_('ok/missing-then-no-dd-then-command', [man('missing'), man('no_dd'), cmd], [resp('tool 2.5')])
case_('ok/no-dd-then-full-date', [man('no_dd'), man('full_date')])

puts JSON.pretty_generate(
  'provenance' => {
    'oracle' => 'ukiryu ab6df353c6dc2d7faf7817a86aafcafa82579896',
    'source' => 'lib/ukiryu/version_detector.rb (VersionDetector.detect_with_methods) with the real ' \
                'lib/ukiryu/man_page_parser.rb (ManPageParser.parse_date)',
    'ruby' => RUBY_DESCRIPTION,
    'date_gem' => Date::VERSION,
    'capture_script' => 'ukiryu/tests/man_detection_golden/capture_man_detection.rb',
    'stubbed' => 'Executor.execute, Shell.detect, Platform.detect (:linux)'
  },
  'today' => { 'year' => Date.today.year, 'mon' => Date.today.mon, 'mday' => Date.today.mday },
  'pages' => PAGES.transform_keys { virt(_1) }.transform_values { hex(_1) },
  'directories' => [virt('a_directory')],
  'cases' => CASES
)
