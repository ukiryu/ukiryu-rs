# frozen_string_literal: true

require 'digest'
require 'json'
require 'rbconfig'
require_relative 'codec'

module UkiryuOracle
  # Reads, executes and compares corpus rows. Shared by capture, verify and
  # the negative-control self test.
  module Runner
    KINDS = %w[resolve build escape validate lint schema cli-output yaml].freeze
    PLATFORMS = %w[linux macos windows].freeze
    # Capabilities a consuming engine may lack. The Ruby oracle provides all.
    CAPABILITIES = %w[fs process cli].freeze
    OPS = %w[shell.escape shell.quote shell.join shell.env_var shell.format_path type.validate build.args
             resolve.extract_version resolve.version_spec resolve.range resolve.shell_detect lint.string
             schema.validate_profile schema.definition_validate yaml.load yaml.model_load yaml.lutaml_roundtrip
             cli.run].freeze
    ROW_KEYS = %w[id kind op api input expect compare error_match applicability sources decision note].freeze

    module_function

    def host_platform
      case RbConfig::CONFIG['host_os']
      when /linux/ then 'linux'
      when /darwin/ then 'macos'
      when /mswin|mingw|cygwin/ then 'windows'
      else RbConfig::CONFIG['host_os']
      end
    end

    def case_files(dir)
      Dir[File.join(dir, 'cases', '*.jsonl')].sort
    end

    # Returns [rows, problems]. Problems are integrity failures.
    def load_rows(dir)
      rows = []
      problems = []
      case_files(dir).each do |file|
        File.readlines(file, chomp: true).each_with_index do |line, n|
          next if line.empty?

          begin
            rows << JSON.parse(line)
          rescue JSON::ParserError => e
            problems << "#{File.basename(file)}:#{n + 1}: invalid JSON (#{e.message.lines.first.strip})"
          end
        end
      end
      [rows, problems]
    end

    def row_problems(row)
      p = []
      id = row['id'] || '<no id>'
      extra = row.keys - ROW_KEYS
      p << "#{id}: unknown keys #{extra.inspect}" unless extra.empty?
      %w[id kind op api input expect compare error_match applicability sources].each do |k|
        p << "#{id}: missing #{k}" unless row.key?(k)
      end
      p << "#{id}: unknown kind #{row['kind'].inspect}" unless KINDS.include?(row['kind'])
      p << "#{id}: unknown op #{row['op'].inspect}" unless OPS.include?(row['op'])
      p << "#{id}: id must start with kind" unless row['id'].to_s.start_with?("#{row['kind']}.")
      exp = row['expect']
      unless exp.is_a?(Hash) && exp.keys.size == 1 && %w[ok error].include?(exp.keys.first)
        p << "#{id}: expect must be {ok: ...} or {error: {class, message}}"
      end
      p << "#{id}: compare must be structural" unless row['compare'] == 'structural'
      p << "#{id}: bad error_match" unless %w[class class_and_message].include?(row['error_match'])
      app = row['applicability']
      if !app.is_a?(Hash) || !app['platforms'].is_a?(Array) || app['platforms'].empty? ||
         !(app['platforms'] - PLATFORMS).empty? || !app['requires'].is_a?(Array) ||
         !(app['requires'] - CAPABILITIES).empty?
        p << "#{id}: applicability needs non-empty known platforms and known requires"
      end
      p
    end

    def ids_digest(ids)
      Digest::SHA256.hexdigest(ids.sort.join("\n"))
    end

    def applicable?(row, platform, capabilities)
      app = row['applicability']
      app['platforms'].include?(platform) && (app['requires'] - capabilities).empty?
    end

    # Executes one row's op and returns an encoded outcome.
    def execute(row)
      value = Ops.run(row['op'], Codec.decode(row['input']))
      { 'ok' => Codec.encode(Ops.scrub(value)) }
    rescue StandardError, ScriptError => e
      { 'error' => { 'class' => e.class.name, 'message' => Ops.scrub(e.message) } }
    end

    # Returns nil when the outcome matches the expectation, else a reason.
    def mismatch(row, outcome)
      exp = row['expect']
      if exp.key?('ok')
        return "expected ok, got error #{outcome['error'].inspect}" unless outcome.key?('ok')
        return nil if Codec.structural_equal?(exp['ok'], outcome['ok'])

        "value differs: expected #{Codec.canonical_json(exp['ok'])[0, 300]} got #{Codec.canonical_json(outcome['ok'])[0, 300]}"
      else
        return "expected error #{exp['error']['class']}, got ok" unless outcome.key?('error')

        e = exp['error']
        a = outcome['error']
        return "error class differs: expected #{e['class']} got #{a['class']}" unless e['class'] == a['class']
        if row['error_match'] == 'class_and_message' && e['message'].b != a['message'].b
          return "error message differs: expected #{e['message'][0, 200].inspect} got #{a['message'][0, 200].inspect}"
        end

        nil
      end
    end

    # Full verification against the manifest. Returns a report hash.
    def verify(dir, platform: host_platform, capabilities: CAPABILITIES, only_integrity: false)
      manifest = JSON.parse(File.read(File.join(dir, 'manifest.json')))
      rows, problems = load_rows(dir)
      rows.each { |r| problems.concat(row_problems(r)) }
      ids = rows.map { |r| r['id'] }
      dups = ids.tally.select { |_, c| c > 1 }.keys
      problems << "duplicate ids: #{dups.first(10).inspect}" unless dups.empty?
      mc = manifest['cases']
      problems << "case count #{ids.size} != manifest #{mc['total']}" unless ids.size == mc['total']
      problems << 'case id set differs from manifest ids_sha256' unless ids_digest(ids) == mc['ids_sha256']
      case_files(dir).each do |f|
        rel = "cases/#{File.basename(f)}"
        want = mc['files'][rel]
        got = Digest::SHA256.file(f).hexdigest
        problems << "#{rel}: sha256 #{got} != manifest #{want.inspect}" unless want == got
      end
      (mc['files'].keys - case_files(dir).map { |f| "cases/#{File.basename(f)}" }).each do |rel|
        problems << "#{rel}: listed in manifest but missing"
      end
      manifest['inputs'].each do |inp|
        path = File.join(dir, 'inputs', inp['path'])
        got = File.exist?(path) ? Digest::SHA256.file(path).hexdigest : 'missing'
        problems << "inputs/#{inp['path']}: sha256 #{got} != #{inp['sha256']}" unless got == inp['sha256']
      end
      report = { 'platform' => platform, 'total' => ids.size, 'integrity_problems' => problems,
                 'executed' => [], 'failed' => [], 'not_applicable' => [] }
      return report if only_integrity

      rows.each do |row|
        unless applicable?(row, platform, capabilities)
          report['not_applicable'] << row['id']
          next
        end
        report['executed'] << row['id']
        reason = mismatch(row, execute(row))
        report['failed'] << { 'id' => row['id'], 'reason' => reason } if reason
      end
      want = manifest.dig('applicable', platform)
      if want.nil?
        problems << "manifest has no applicable count for platform #{platform}"
      elsif report['executed'].size != want
        problems << "executed #{report['executed'].size} cases on #{platform}, manifest pins #{want}"
      end
      report
    end

    def passed?(report)
      report['integrity_problems'].empty? && report['failed'].empty?
    end
  end
end
