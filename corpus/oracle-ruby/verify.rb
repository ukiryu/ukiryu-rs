# frozen_string_literal: true

# Ruby consumer of the conformance corpus: replays every applicable case
# against the pinned Ruby implementation and fails on any mismatch, integrity
# problem or count drift.
#
#   UKIRYU_CONFORMANCE_DIR=../conformance bundle exec ruby verify.rb [--report FILE] [--integrity-only]
#
# There is no skip switch. A case is either applicable on this platform (and
# executed) or declares in its own row that it is not.

require 'bundler/setup'
require 'json'
require 'ukiryu'
require_relative 'lib/codec'
require_relative 'lib/ops'
require_relative 'lib/runner'

dir = File.expand_path(ENV.fetch('UKIRYU_CONFORMANCE_DIR') { File.expand_path('../conformance', __dir__) })
ENV['UKIRYU_CONFORMANCE_DIR'] = dir
report_path = ARGV.include?('--report') ? ARGV[ARGV.index('--report') + 1] : nil
UkiryuOracle::Ops::Sandbox.setup!
report = UkiryuOracle::Runner.verify(dir, only_integrity: ARGV.include?('--integrity-only'))
File.write(report_path, JSON.pretty_generate(report)) if report_path

report['integrity_problems'].first(50).each { |p| warn "INTEGRITY: #{p}" }
report['failed'].first(50).each { |f| warn "FAIL #{f['id']}: #{f['reason']}" }
puts format('corpus %<dir>s on %<pf>s: total=%<t>d executed=%<e>d failed=%<f>d not_applicable=%<n>d integrity_problems=%<i>d',
            dir: dir, pf: report['platform'], t: report['total'], e: report['executed'].size,
            f: report['failed'].size, n: report['not_applicable'].size, i: report['integrity_problems'].size)
exit(UkiryuOracle::Runner.passed?(report) ? 0 : 1)
