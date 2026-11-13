# frozen_string_literal: true

# Measures the bytes the pinned Ruby DocumentationGenerator.generate_to_file
# (File.write) produces on the host OS, against the LF target text recorded
# in oracle_cases.json (captured on Linux and matched byte for byte by the
# Rust port).
#
# Usage (from corpus/oracle-ruby, locked Gemfile):
#   bundle exec ruby file_write_capture.rb <oracle_cases.json> <out_dir> [<record.json>]
#
# For every captured successful `generate` call it writes the file with the
# real oracle method, reads the bytes back in binary mode and classifies them:
#   identical      - bytes equal the LF target
#   lf_to_crlf     - bytes equal the target with every LF byte (0x0A) replaced
#                    by CR LF (0x0D 0x0A); CR bytes already in the target,
#                    including CRs embedded in values, are kept as they are
#   other          - anything else (fails)
# No CR is ever stripped; the comparison is byte for byte after that single
# transform. All cases must share one classification. When <record.json> is
# given, the measured host result must also equal the committed record.
require 'json'
require 'yaml'
require 'digest'
require 'fileutils'
require 'ukiryu/definition/documentation_generator'

cases_path, out_dir, record_path = ARGV
abort 'usage: file_write_capture.rb <oracle_cases.json> <out_dir> [<record.json>]' unless cases_path && out_dir

oracle = JSON.parse(File.read(cases_path, mode: 'rb', encoding: 'UTF-8'))
expected_sha = 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
abort "oracle sha mismatch: #{oracle['provenance']['oracle_sha']}" unless oracle['provenance']['oracle_sha'] == expected_sha

G = Ukiryu::Definition::DocumentationGenerator
FileUtils.mkdir_p(out_dir)
files_dir = File.join(out_dir, 'files')
FileUtils.mkdir_p(files_dir)

def classify(written, target)
  return 'identical' if written == target
  return 'lf_to_crlf' if written == target.gsub("\n", "\r\n")

  'other'
end

results = []
failures = []
n = 0
oracle['cases'].each do |c|
  c['results'].each do |r|
    next unless r['op'] == 'generate' && r['result'].key?('ok')

    n += 1
    target = r['result']['ok'].b
    input = YAML.safe_load(c['yaml'], symbolize_names: true)
    text = G.generate(input, format: r['format'].to_sym).b
    failures << "#{c['name']} #{r['format']}: generate text differs from LF target" unless text == target

    path = File.join(files_dir, format('%03d-%s.out', n, r['format']))
    G.generate_to_file(input, path, format: r['format'].to_sym)
    written = File.binread(path)
    kind = classify(written, target)
    failures << "#{c['name']} #{r['format']}: written bytes match neither identical nor lf_to_crlf" if kind == 'other'
    results << {
      'case' => c['name'], 'format' => r['format'], 'file' => File.basename(path), 'classification' => kind,
      'target_bytes' => target.bytesize, 'target_lf' => target.count("\n"), 'target_cr' => target.count("\r"),
      'written_bytes' => written.bytesize, 'written_lf' => written.count("\n"), 'written_cr' => written.count("\r"),
      'written_sha256' => Digest::SHA256.hexdigest(written)
    }
  end
end

kinds = results.map { |x| x['classification'] }.uniq
failures << "inconsistent classifications: #{kinds.inspect}" unless kinds.size == 1

# The one case whose values embed CR LF: record its exact written bytes.
embedded = results.select { |x| x['case'] == 'crlf_in_values' }.map do |x|
  bytes = File.binread(File.join(files_dir, x['file']))
  { 'format' => x['format'], 'written_hex' => bytes.unpack1('H*') }
end

report = {
  'host_os' => RbConfig::CONFIG['host_os'],
  'ruby' => RUBY_DESCRIPTION,
  'psych' => Psych::VERSION,
  'default_external' => Encoding.default_external.name,
  'oracle_sha' => expected_sha,
  'method' => 'Ukiryu::Definition::DocumentationGenerator.generate_to_file (File.write)',
  'calls' => results.size,
  'classification' => kinds.size == 1 ? kinds.first : 'mixed',
  'embedded_crlf_case' => embedded,
  'results' => results
}
File.write(File.join(out_dir, 'report.json'), "#{JSON.pretty_generate(report)}\n", mode: 'wb')
puts "host_os=#{report['host_os']} calls=#{results.size} classification=#{report['classification']}"
embedded.each { |e| puts "crlf_in_values #{e['format']}: #{e['written_hex']}" }

if record_path
  record = JSON.parse(File.read(record_path, mode: 'rb', encoding: 'UTF-8'))
  host = record.fetch('hosts').find { |h| h['host_os'] == report['host_os'] }
  if host.nil?
    failures << "no committed record for host_os #{report['host_os']}"
  else
    failures << "classification #{report['classification']} != record #{host['classification']}" unless host['classification'] == report['classification']
    failures << 'embedded CR LF bytes differ from record' unless host['embedded_crlf_case'] == embedded
    recorded = host.fetch('written_sha256')
    measured = results.to_h { |x| ["#{x['case']}|#{x['format']}", x['written_sha256']] }
    failures << 'written sha256 set differs from record' unless recorded == measured
  end
end

unless failures.empty?
  warn failures.first(25).join("\n")
  exit 1
end
