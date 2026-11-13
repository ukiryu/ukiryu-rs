# frozen_string_literal: true

# Negative controls: proves the runner rejects a corpus that is wrong.
# Each control copies the committed corpus, applies one deliberate defect and
# requires verification to fail with the expected reason. The unmodified copy
# must pass. Any control that passes verification fails this script.

require 'bundler/setup'
require 'digest'
require 'fileutils'
require 'json'
require 'tmpdir'
require 'ukiryu'
require_relative 'lib/codec'
require_relative 'lib/ops'
require_relative 'lib/runner'

SRC = File.expand_path(ENV.fetch('UKIRYU_CONFORMANCE_DIR') { File.expand_path('../conformance', __dir__) })
R = UkiryuOracle::Runner

# Controls only change expectations and bookkeeping, never inputs, so each
# distinct (op, input) executes once and later controls reuse its outcome.
module MemoExecute
  MEMO = {}
  def execute(row)
    MEMO[[row['op'], UkiryuOracle::Codec.canonical_json(row['input'])]] ||= super
  end
end
R.singleton_class.prepend(MemoExecute)

def edit_rows(dir, kind)
  file = File.join(dir, 'cases', "#{kind}.jsonl")
  rows = File.readlines(file).map { |l| JSON.parse(l) }
  rows = yield(rows)
  File.write(file, rows.map { |r| "#{JSON.generate(r)}\n" }.join)
end

def reseal(dir)
  m = JSON.parse(File.read(File.join(dir, 'manifest.json')))
  R.case_files(dir).each { |f| m['cases']['files']["cases/#{File.basename(f)}"] = Digest::SHA256.file(f).hexdigest }
  File.write(File.join(dir, 'manifest.json'), JSON.pretty_generate(m))
end

def first_ok(rows) = rows.index { |r| r['expect'].key?('ok') && r['expect']['ok'].is_a?(String) && !r['expect']['ok'].empty? }
def first_err(rows) = rows.index { |r| r['expect'].key?('error') }

CONTROLS = {
  'wrong-expected-value' => [/value differs/, lambda { |d|
    edit_rows(d, 'escape') { |rs| i = first_ok(rs); rs[i]['expect']['ok'] += 'X'; rs }
    reseal(d)
  }],
  'wrong-needs-quoting-boolean' => [/value differs/, lambda { |d|
    edit_rows(d, 'escape') do |rs|
      i = rs.index { |r| r['op'] == 'shell.needs_quoting' && r['expect']['ok'] == true }
      rs[i]['expect']['ok'] = false
      rs
    end
    reseal(d)
  }],
  'wrong-error-class' => [/error class differs/, lambda { |d|
    edit_rows(d, 'validate') { |rs| i = first_err(rs); rs[i]['expect']['error']['class'] = 'RuntimeError'; rs }
    reseal(d)
  }],
  'wrong-error-message' => [/error message differs/, lambda { |d|
    edit_rows(d, 'validate') { |rs| i = first_err(rs); rs[i]['expect']['error']['message'] += '!'; rs }
    reseal(d)
  }],
  'wrong-host-platform-field' => [/error message differs/, lambda { |d|
    edit_rows(d, 'resolve') do |rs|
      row = rs.find { |r| r['id'] == 'resolve.shell_detect.shell-env.empty' }
      row['expect']['error']['message'] = row['expect']['error']['message'].sub('${RUBY_PLATFORM}', 'wrong-platform')
      rs
    end
    reseal(d)
  }],
  'expected-error-got-ok' => [/expected error/, lambda { |d|
    edit_rows(d, 'escape') { |rs| i = first_ok(rs); rs[i]['expect'] = { 'error' => { 'class' => 'X', 'message' => 'x' } }; rs }
    reseal(d)
  }],
  'missing-case' => [/case count|ids_sha256/, lambda { |d|
    edit_rows(d, 'lint') { |rs| rs.drop(1) }
    reseal(d)
  }],
  'duplicate-case' => [/duplicate ids/, lambda { |d|
    edit_rows(d, 'lint') { |rs| rs + [rs.first] }
    reseal(d)
  }],
  'unexpected-extra-case' => [/case count|ids_sha256/, lambda { |d|
    edit_rows(d, 'lint') { |rs| x = JSON.parse(JSON.generate(rs.first)); x['id'] += '-extra'; rs + [x] }
    reseal(d)
  }],
  'unsealed-edit' => [/sha256/, lambda { |d|
    edit_rows(d, 'escape') { |rs| rs.reverse }
  }],
  'tampered-input' => [/inputs\/.*sha256/, lambda { |d|
    File.open(File.join(d, 'inputs', 'profiles', 'inkscape_1.0.yaml'), 'a') { |f| f.write("\n") }
  }],
  'platform-silently-omitted' => [/executed \d+ cases on/, lambda { |d|
    other = (R::PLATFORMS - [R.host_platform]).first
    edit_rows(d, 'escape') { |rs| rs[0]['applicability']['platforms'] = [other]; rs }
    reseal(d)
  }],
  'unknown-op' => [/unknown op/, lambda { |d|
    edit_rows(d, 'escape') { |rs| rs[0]['op'] = 'shell.frobnicate'; rs }
    reseal(d)
  }],
  'empty-applicability' => [/applicability/, lambda { |d|
    edit_rows(d, 'escape') { |rs| rs[0]['applicability']['platforms'] = []; rs }
    reseal(d)
  }]
}.freeze

def verify(dir)
  ENV['UKIRYU_CONFORMANCE_DIR'] = dir
  UkiryuOracle::Ops::Sandbox.setup!
  rep = R.verify(dir)
  [R.passed?(rep), (rep['integrity_problems'] + rep['failed'].map { |f| f['reason'] }).join("\n")]
end

failures = 0
Dir.mktmpdir('ukiryu-selftest') do |tmp|
  base = File.join(tmp, 'clean')
  FileUtils.cp_r(SRC, base)
  ok, why = verify(base)
  puts "control clean-copy: #{ok ? 'PASS (verifies)' : "FAILED: #{why.lines.first(5).join}"}"
  failures += 1 unless ok
  # A bad actual diagnostic must not be erased by a broad Platform regexp.
  row = R.load_rows(base).first.find { |r| r['id'] == 'resolve.shell_detect.shell-env.empty' }
  wrong = row['expect']['error']['message'].sub('${RUBY_PLATFORM}', '__CORPUS_WRONG_PLATFORM__')
  message = UkiryuOracle::Ops.scrub_error(Ukiryu::Errors::UnknownShellError.new(wrong), op: row['op'])
  reason = R.mismatch(row, { 'error' => { 'class' => row['expect']['error']['class'], 'message' => message } })
  bound = message.include?('__CORPUS_WRONG_PLATFORM__') && reason&.match?(/error message differs/)
  failures += 1 unless bound
  puts "control normalizer-bound-to-runtime-platform: #{bound ? 'PASS (wrong actual value rejected)' : 'FAILED'}"
  CONTROLS.each do |name, (pattern, mutate)|
    d = File.join(tmp, name)
    FileUtils.cp_r(SRC, d)
    mutate.call(d)
    ok, why = verify(d)
    good = !ok && why.match?(pattern)
    failures += 1 unless good
    puts "control #{name}: #{good ? 'PASS (rejected)' : "NOT REJECTED as expected (passed=#{ok}) #{why.lines.first(3).join.strip}"}"
  end
end
puts "selftest: #{CONTROLS.size + 2} controls, #{failures} failures"
exit(failures.zero? ? 0 : 1)
