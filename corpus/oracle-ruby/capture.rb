# frozen_string_literal: true

# Regenerates corpus/conformance from the pinned Ruby implementation.
#
#   UKIRYU_RUBY_SRC=<checkout of ukiryu/ukiryu at the pinned commit> \
#     bundle exec ruby capture.rb [--check]
#
# Inputs are copied byte-for-byte from the pinned source; every case row is
# generated from those inputs and the expectation is whatever the pinned Ruby
# code returns or raises. --check regenerates into a temporary directory and
# fails if any committed byte differs.

require 'bundler/setup'
require 'digest'
require 'fileutils'
require 'json'
require 'tmpdir'
require 'ukiryu'
require_relative 'lib/codec'
require_relative 'lib/ops'
require_relative 'lib/runner'

module UkiryuOracle
  module Capture
    module_function

    ALL = Runner::PLATFORMS
    UNIX = %w[linux macos].freeze

    # Source tree → corpus inputs. Paths are relative to the pinned checkout.
    INPUT_MAP = {
      'spec/fixtures/register' => 'register',
      'spec/fixtures/profiles' => 'profiles',
      'spec/fixtures/tool.schema.yaml' => 'fixture-schema/tool.schema.yaml',
      'lib/ukiryu/schemas/v1' => 'schemas/v1',
      # The remaining tracked YAML of the pinned checkout, parsed by yaml.load
      # only, so every tracked YAML file has a parse row.
      '.github/workflows' => 'repo-yaml/github-workflows',
      'docs/_config.yml' => 'repo-yaml/docs/_config.yml',
      '.rubocop.yml' => 'repo-yaml/rubocop.yml',
      '.rubocop_todo.yml' => 'repo-yaml/rubocop_todo.yml'
    }.freeze

    def source_dir
      ENV.fetch('UKIRYU_RUBY_SRC') { Gem.loaded_specs.fetch('ukiryu').full_gem_path }
    end

    def source_commit
      spec = Gem.loaded_specs.fetch('ukiryu')
      rev = spec.source.respond_to?(:revision) ? spec.source.revision : nil
      rev || `git -C #{source_dir} rev-parse HEAD`.strip
    end

    # Authored register for CLI rows that report executable discovery. Its
    # only tool has an executable name and search paths that exist on no
    # host, so "not installed" is a controlled input, not a host accident.
    SYNTHETIC_REGISTER = {
      'interfaces/absenttool.yaml' => <<~YAML,
        ---
        ukiryu_schema: '1.0'
        name: absenttool
        version: '1.0'
        display_name: Absent Tool Interface
        actions:
        - name: process
          description: Process input
          signature:
            inputs:
              inputs:
              - name: input
                type: file
                required: true
              options:
              - name: level
                type: integer
              flags:
              - name: verbose
                type: boolean
      YAML
      'tools/absenttool/index.yaml' => <<~YAML,
        ---
        ukiryu_schema: '1.0'
        name: absenttool
        interface: absenttool
        implementations:
        - name: default
          display_name: Absent Tool
          detection:
            pattern: absenttool ([\d.]+)
          version_scheme: semantic
          versions:
          - equals: '1.0'
            file: 1.0.yaml
          default: 1.0.yaml
      YAML
      'tools/absenttool/default/1.0.yaml' => <<~YAML
        ---
        ukiryu_schema: '1.0'
        name: absenttool
        display_name: Absent Tool (corpus control)
        version: '1.0'
        aliases: []
        version_detection:
          command: "--version"
          pattern: absenttool ([\d.]+)
        search_paths:
          macos:
          - "/nonexistent/ukiryu-corpus/bin/ukiryu-corpus-absent-tool"
          linux:
          - "/nonexistent/ukiryu-corpus/bin/ukiryu-corpus-absent-tool"
          windows:
          - "C:/nonexistent/ukiryu-corpus/ukiryu-corpus-absent-tool.exe"
        implements: absenttool/1.0
        execution_profiles:
        - name: default
          platforms:
          - macos
          - linux
          - windows
          shells:
          - bash
          - zsh
          - sh
          - fish
          - powershell
          - cmd
          option_style: double_dash_equals
          executable_name: ukiryu-corpus-absent-tool
          commands:
          - name: process
            description: Process input
            arguments:
            - name: input
              type: file
              position: last
              required: true
            options:
            - name: level
              cli: "--level"
              type: integer
            flags:
            - name: verbose
              cli: "--verbose"
      YAML
    }.freeze

    # The CLI's implicit remote-schema lookup was part of the original capture.
    # Pin that exact external input locally so network failures cannot silently
    # turn the recorded rejection into structural-only success.
    PINNED_REMOTE_SCHEMA = {
      'path' => 'external-schema/tool.schema.yaml',
      'source_path' => 'https://github.com/ukiryu/schemas/blob/744630fc341681fae75cf2629a533717f1ff3ede/v1/tool.schema.yaml',
      'sha256' => 'a269b860f7f6ce28e7c70ac02c1e80c3e843be466ebf40bfc090b7330ca43285'
    }.freeze

    def copy_inputs(out)
      rows = []
      schema_bytes = File.binread(File.expand_path("../conformance/inputs/#{PINNED_REMOTE_SCHEMA['path']}", __dir__))
      unless Digest::SHA256.hexdigest(schema_bytes) == PINNED_REMOTE_SCHEMA['sha256']
        raise 'pinned external schema bytes do not match their source digest'
      end
      schema_target = File.join(out, 'inputs', PINNED_REMOTE_SCHEMA['path'])
      FileUtils.mkdir_p(File.dirname(schema_target))
      File.binwrite(schema_target, schema_bytes)
      rows << PINNED_REMOTE_SCHEMA
      SYNTHETIC_REGISTER.each do |rel, body|
        target = File.join(out, 'inputs', 'synthetic-register', rel)
        FileUtils.mkdir_p(File.dirname(target))
        File.write(target, body)
        rows << { 'path' => "synthetic-register/#{rel}", 'source_path' => 'authored: corpus/oracle-ruby/capture.rb',
                  'sha256' => Digest::SHA256.hexdigest(body) }
      end
      INPUT_MAP.each do |src_rel, dst_rel|
        src = File.join(source_dir, src_rel)
        files = File.directory?(src) ? Dir[File.join(src, '**', '*')].select { |f| File.file?(f) } : [src]
        files.sort.each do |f|
          rel = f.delete_prefix("#{source_dir}/")
          dst_path = File.join(dst_rel, rel.delete_prefix(src_rel).delete_prefix('/'))
          dst_path = dst_rel if f == src
          target = File.join(out, 'inputs', dst_path)
          FileUtils.mkdir_p(File.dirname(target))
          FileUtils.cp(f, target)
          rows << { 'path' => dst_path, 'source_path' => rel, 'sha256' => Digest::SHA256.file(f).hexdigest }
        end
      end
      rows.sort_by { |r| r['path'] }
    end

    # ------------------------------------------------------------------
    # Row helpers

    def slug(text)
      s = text.to_s.downcase.gsub(/[^a-z0-9]+/, '-').gsub(/\A-|-\z/, '')
      s.empty? ? 'empty' : s[0, 48]
    end

    def row(id:, kind:, op:, api:, input:, platforms: ALL, requires: [], error_match: 'class_and_message',
            sources: [], decision: nil, note: nil)
      r = { 'id' => id, 'kind' => kind, 'op' => op, 'api' => api, 'input' => Codec.encode(input),
            'compare' => 'structural', 'error_match' => error_match,
            'applicability' => { 'platforms' => platforms, 'requires' => requires }, 'sources' => sources }
      r['decision'] = decision if decision
      r['note'] = note if note
      r
    end

    # ------------------------------------------------------------------
    # escape: per-shell quoting/joining matrix

    STRINGS = [
      ['empty', ''], ['simple', 'simple'], ['space', 'with space'], ['tab', "tab\there"],
      ['newline', "new\nline"], ['single-quote', "it's"], ['double-quote', 'say "hi"'],
      ['backslash', 'back\\slash'], ['dollar-var', '$HOME'], ['brace-var', '${VAR}'],
      ['backtick', '`cmd`'], ['subshell', '$(cmd)'], ['semicolon', 'a;b'], ['ampersand', 'a&b'],
      ['pipe', 'a|b'], ['redirect', 'a>b'], ['glob', '*.txt'], ['question', 'what?'],
      ['tilde', '~user'], ['bang', '!bang'], ['percent', '%PATH%'], ['caret', '^caret'],
      ['windows-path', 'C:\\Program Files\\x'], ['unicode', 'unicode é ü 日本'], ['dash-flag', '-flag'],
      ['equals', 'a=b'], ['hash', '#hash'], ['paren', '(paren)'], ['bracket', '[br]'], ['braces', '{a,b}'],
      ['mixed-quotes', %q(it's "both")], ['trailing-backslash', 'end\\']
    ].freeze

    JOINS = [
      ['echo-hello', 'echo', ['hello']],
      ['convert-spaces', '/usr/bin/convert', ['in put.png', '-resize', '50%', "out'1.png"]],
      ['windows-exe', 'C:\\Program Files\\tool.exe', ['a b', 'c"d']],
      ['no-args', 'tool', []],
      ['empty-arg', 'tool', ['']],
      ['jq-filter', 'jq', ['.a | .b', '$x']],
      ['glob-and-vars', 'ls', ['*.txt', '$HOME', '`id`']]
    ].freeze

    ENV_NAMES = %w[HOME PATH MY_VAR_1].freeze
    PATHS = [['unix-abs', '/usr/bin/x'], ['windows-abs', 'C:\\dir\\file.txt'], ['relative', 'relative/path'],
             ['space', 'with space/a b'], ['empty', '']].freeze

    def escape_rows
      rows = []
      Ops::SHELLS.each do |shell, klass|
        STRINGS.each do |label, s|
          %w[escape quote].each do |m|
            rows << row(id: "escape.#{shell}.#{m}.#{label}", kind: 'escape', op: "shell.#{m}",
                        api: "Ukiryu::Shell::#{klass}##{m}", input: { 'shell' => shell, 'string' => s })
          end
        end
        JOINS.each do |label, exe, args|
          rows << row(id: "escape.#{shell}.join.#{label}", kind: 'escape', op: 'shell.join',
                      api: "Ukiryu::Shell::#{klass}#join",
                      input: { 'shell' => shell, 'executable' => exe, 'args' => args })
        end
        ENV_NAMES.each do |n|
          rows << row(id: "escape.#{shell}.env_var.#{slug(n)}", kind: 'escape', op: 'shell.env_var',
                      api: "Ukiryu::Shell::#{klass}#env_var", input: { 'shell' => shell, 'name' => n })
        end
        PATHS.each do |label, pth|
          rows << row(id: "escape.#{shell}.format_path.#{label}", kind: 'escape', op: 'shell.format_path',
                      api: "Ukiryu::Shell::#{klass}#format_path", input: { 'shell' => shell, 'path' => pth })
        end
      end
      rows
    end

    # ------------------------------------------------------------------
    # validate: Ukiryu::Type.validate accept/reject matrix

    VALIDATE = [
      ['string-ok', 'abc', :string, {}], ['string-empty', '', :string, {}],
      ['string-empty-allowed', '', :string, { allow_empty: true }],
      ['string-from-integer', 42, :string, {}],
      ['string-pattern-ok', 'abc123', :string, { pattern: /\A[a-z]+\d+\z/ }],
      ['string-pattern-bad', 'ABC', :string, { pattern: /\A[a-z]+\z/ }],
      ['string-type-as-string', 'x', 'string', {}],
      ['file-ok', 'in put.png', :file, {}], ['file-empty', '', :file, {}],
      ['integer-int', 5, :integer, {}], ['integer-string', ' 12 ', :integer, {}],
      ['integer-hex-string', '0x1f', :integer, {}], ['integer-octal-string', '010', :integer, {}],
      ['integer-underscore', '1_000', :integer, {}], ['integer-float-string', '1.5', :integer, {}],
      ['integer-word', 'abc', :integer, {}], ['integer-nil', nil, :integer, {}],
      ['integer-float', 2.9, :integer, {}],
      ['integer-range-in', 5, :integer, { range: [1, 10] }], ['integer-range-low', 0, :integer, { range: [1, 10] }],
      ['integer-range-high', 11, :integer, { range: [1, 10] }], ['integer-range-edge', 10, :integer, { range: [1, 10] }],
      ['integer-min-bad', 2, :integer, { min: 3 }], ['integer-max-bad', 9, :integer, { max: 8 }],
      ['float-ok', '1.25', :float, {}], ['float-int', 3, :float, {}], ['float-exp', '1e3', :float, {}],
      ['float-bad', 'x1', :float, {}], ['float-range-bad', 2.5, :float, { range: [0, 1] }],
      ['symbol-string', 'Fast', :symbol, {}], ['symbol-values-ok', 'fast', :symbol, { values: %w[fast slow] }],
      ['symbol-values-bad', 'medium', :symbol, { values: %w[fast slow] }],
      ['boolean-true', true, :boolean, {}], ['boolean-yes', 'yes', :boolean, {}], ['boolean-on', 'on', :boolean, {}],
      ['boolean-0', '0', :boolean, {}], ['boolean-empty', '', :boolean, {}], ['boolean-TRUE', 'TRUE', :boolean, {}],
      ['boolean-int-1', 1, :boolean, {}], ['boolean-bad', 'maybe', :boolean, {}],
      ['uri-ok', 'https://example.com/a?b=c', :uri, {}], ['uri-relative', 'a/b', :uri, {}],
      ['uri-empty', '', :uri, {}], ['uri-bad', 'http://exa mple.com', :uri, {}],
      ['datetime-iso', '2024-01-17T10:20:30Z', :datetime, {}], ['datetime-bad', 'not a date', :datetime, {}],
      ['hash-ok', { 'a' => 1 }, :hash, {}], ['hash-bad', 'a=1', :hash, {}],
      ['hash-keys-bad', { 'a' => 1, 'z' => 2 }, :hash, { keys: %w[a b] }],
      ['array-wrap-scalar', 'x', :array, {}], ['array-ok', %w[a b], :array, {}],
      ['array-min-bad', %w[a], :array, { min: 2 }], ['array-max-bad', %w[a b c], :array, { max: 2 }],
      ['array-size-bad', %w[a b], :array, { size: 3 }], ['array-size-set-ok', %w[a b], :array, { size: [2, 4] }],
      ['array-size-set-bad', %w[a b c], :array, { size: [2, 4] }],
      ['array-of-integer', %w[1 2], :array, { of: :integer }],
      ['array-of-integer-bad', %w[1 x], :array, { of: :integer }],
      ['unknown-type', 'x', :color, {}], ['type-hash-form', '7', { name: :integer }, {}]
    ].freeze

    def validate_rows
      VALIDATE.map do |label, value, type, options|
        row(id: "validate.type.#{label}", kind: 'validate', op: 'type.validate', api: 'Ukiryu::Type.validate',
            input: { 'value' => value, 'type' => type, 'options' => options })
      end
    end

    # ------------------------------------------------------------------
    # build: argv assembly from register profiles

    PROFILE_TRIES = [%w[linux bash], %w[macos zsh], %w[windows powershell], %w[windows cmd]].freeze

    def sample_value(d, variadic: false)
      type = d.respond_to?(:type) ? d.type.to_s : 'string'
      values = d.respond_to?(:values) ? Array(d.values) : []
      v = if !values.empty? then values.first
          elsif type == 'integer' then (d.respond_to?(:range) && d.range&.first) || 7
          elsif type == 'float' then 1.5
          elsif type == 'boolean' then true
          elsif type == 'array' then of = d.respond_to?(:of) ? d.of.to_s : ''
                                     of == 'integer' ? [1, 2] : ['a b', 'c']
          elsif type == 'file' then 'in put.png'
          else "val ue'1"
          end
      variadic && !v.is_a?(Array) ? [v, 'second file.txt'] : v
    end

    def build_rows
      rows = []
      reg = File.join(Ops.corpus_dir, 'inputs', 'register', 'tools')
      Dir[File.join(reg, '*', 'index.yaml')].sort.each do |index_path|
        tool = File.basename(File.dirname(index_path))
        index = YAML.safe_load(File.read(index_path))
        Array(index['implementations']).each do |impl|
          files = Array(impl['versions']).map { |v| v['file'] }.compact.uniq
          files.each do |file|
            vpath = File.join(reg, tool, impl['name'], file)
            src = File.exist?(vpath) ? ["register/tools/#{tool}/#{impl['name']}/#{file}"] : []
            base = { 'tool' => tool, 'implementation' => impl['name'], 'file' => file }
            chosen = nil
            PROFILE_TRIES.each do |pf, sh|
              Ops::Sandbox.reset!
              t = Ops.load_tool(base.merge('platform' => pf, 'shell' => sh))
              chosen = [pf, sh, t]
              break
            rescue StandardError
              next
            end
            prefix = "build.#{tool}.#{slug(impl['name'])}.#{slug(file.delete_suffix('.yaml'))}"
            unless chosen
              rows << row(id: "#{prefix}.load", kind: 'build', op: 'build.args', api: 'Ukiryu::Tool#build_args',
                          input: base.merge('platform' => 'linux', 'shell' => 'bash', 'command' => 'none',
                                            'params' => {}), sources: src,
                          note: 'version file has no loadable profile for any tried platform/shell')
              next
            end
            pf, sh, t = chosen
            t.commands.each do |cmd|
              args = Array(cmd.arguments)
              required = args.select(&:required).to_h { |a| [a.name, sample_value(a, variadic: a.variadic)] }
              all_args = args.to_h { |a| [a.name, sample_value(a, variadic: a.variadic)] }
              sets = [['empty', {}], ['required', required], ['all-arguments', all_args]]
              (Array(cmd.options) + Array(cmd.post_options)).each do |o|
                sets << ["option-#{slug(o.name)}", required.merge(o.name => sample_value(o))]
              end
              Array(cmd.flags).each do |f|
                sets << ["flag-#{slug(f.name)}-true", required.merge(f.name => true)]
                sets << ["flag-#{slug(f.name)}-false", required.merge(f.name => false)]
              end
              seen = {}
              sets.each do |label, params|
                key = Codec.canonical_json(Codec.encode(params))
                next if seen[key]

                seen[key] = true
                rows << row(id: "#{prefix}.#{slug(cmd.name)}.#{label}", kind: 'build', op: 'build.args',
                            api: 'Ukiryu::Tool#build_args',
                            input: base.merge('platform' => pf, 'shell' => sh, 'command' => cmd.name,
                                              'params' => params), sources: src)
              end
            end
          end
        end
      end
      rows
    end

    # ------------------------------------------------------------------
    # resolve: version extraction, range matching, spec routing, shell detection

    OUTPUTS = [
      ['name-space', 'TOOL 1.2.3'], ['name-dash', 'TOOL-1.6'], ['version-word', 'version 10.0'],
      ['imagemagick', "Version: ImageMagick 7.1.1-21 Q16-HDRI x86_64\nCopyright: (C)"],
      ['ghostscript', '10.02.1'], ['iputils', 'ping from iputils 20240117'],
      ['inkscape', 'Inkscape 1.3.2 (091e20e, 2023-11-25)'], ['busybox', 'BusyBox v1.36.1 (2023-11-07) multi-call binary.'],
      ['gnu', 'gzip 1.12'], ['no-digits', 'no digits here'], ['empty', ''],
      ['invalid-utf8', "tool \xFF\xFE 1.0".b]
    ].freeze

    def detection_patterns
      pats = []
      Dir[File.join(Ops.corpus_dir, 'inputs', 'register', 'tools', '**', '*.yaml')].sort.each do |f|
        rel = f.delete_prefix(File.join(Ops.corpus_dir, 'inputs') + '/')
        YAML.safe_load(File.read(f)).then do |doc|
          walk = lambda do |node|
            case node
            when Hash
              pats << [node['pattern'], rel] if node['pattern'].is_a?(String)
              node.each_value { |v| walk.call(v) }
            when Array then node.each { |v| walk.call(v) }
            end
          end
          walk.call(doc)
        end
      end
      pats.uniq(&:first)
    end

    RANGE_MATRIX = {
      'semantic' => { 'boundary' => '1.2.3', 'low' => '1.0.0', 'high' => '2.0.0',
                      'versions' => %w[1.2.2 1.2.3 1.2.4 1.2.3-rc.1 1.10.0 2.0.0 1.2 1 0.9.9 abc] },
      'calver' => { 'boundary' => '2024.01.17', 'low' => '2024.01.01', 'high' => '2024.12.31',
                    'versions' => %w[2024.01.16 2024.01.17 2024.01.18 2023.12.31 2024.1.17 2025.01.01 abc] }
    }.freeze

    def resolve_rows
      rows = []
      detection_patterns.each_with_index do |(pattern, src), n|
        OUTPUTS.each do |label, out|
          rows << row(id: "resolve.extract_version.p#{format('%02d', n)}.#{label}", kind: 'resolve',
                      op: 'resolve.extract_version', api: 'Ukiryu::Tool::Loader.extract_version_from_pattern (private)',
                      input: { 'output' => out, 'pattern' => pattern }, sources: [src])
        end
      end
      RANGE_MATRIX.each do |scheme, m|
        ranges = {
          'equals' => { 'version' => m['boundary'] }, 'before' => { 'version' => m['boundary'] },
          'after' => { 'version' => m['boundary'] }, 'between' => { 'from' => m['low'], 'to' => m['high'] }
        }
        ranges.each do |range, bounds|
          m['versions'].each do |v|
            rows << row(id: "resolve.range.#{scheme}.#{range}.#{slug(v)}", kind: 'resolve', op: 'resolve.range',
                        api: 'Versionian::VersionRange#matches? via Ukiryu::VersionSchemeResolver',
                        input: { 'scheme' => scheme, 'range' => range, 'bounds' => bounds, 'version' => v })
          end
        end
      end
      reg = File.join(Ops.corpus_dir, 'inputs', 'register', 'tools')
      Dir[File.join(reg, '*', 'index.yaml')].sort.each do |index_path|
        tool = File.basename(File.dirname(index_path))
        index = YAML.safe_load(File.read(index_path))
        Array(index['implementations']).each do |impl|
          cands = %w[0.0.1 99.0]
          Array(impl['versions']).each do |v|
            %w[equals before after].each do |k|
              next unless v[k]

              b = v[k].to_s
              cands += [b, "#{b}.1", b.sub(/\d+\z/) { |d| (d.to_i + 1).to_s }, b.sub(/\d+\z/) { |d| [d.to_i - 1, 0].max.to_s }]
            end
          end
          cands.uniq.each do |c|
            rows << row(id: "resolve.version_spec.#{tool}.#{slug(impl['name'])}.#{slug(c)}", kind: 'resolve',
                        op: 'resolve.version_spec',
                        api: 'Ukiryu::Tool::Loader.find_matching_version_spec (private)',
                        input: { 'tool' => tool, 'implementation' => impl['name'], 'version' => c },
                        sources: ["register/tools/#{tool}/index.yaml"])
          end
        end
      end
      shells = ['/bin/bash', '/usr/bin/zsh', '/usr/local/bin/fish', '/bin/sh', '/bin/dash', '/bin/tcsh', '/bin/csh',
                '/bin/ksh', '/usr/bin/mksh', '/opt/notbash', '/opt/fakezsh', 'bash', '', '/nonexistent/weird']
      shells.each do |s|
        rows << row(id: "resolve.shell_detect.shell-env.#{slug(s)}", kind: 'resolve', op: 'resolve.shell_detect',
                    api: 'Ukiryu::Shell.detect / Ukiryu::Shell.shell_class', platforms: UNIX, decision: 'D2',
                    input: { 'env' => { 'SHELL' => s, 'UKIRYU_TEST_SHELL' => nil } },
                    note: 'Ruby oracle behaviour; suffix matching and fallbacks await owner decision D2')
      end
      [%w[tcsh /bin/bash], %w[nonsense /bin/bash], %w[cmd /bin/bash]].each do |ts, s|
        rows << row(id: "resolve.shell_detect.test-shell.#{slug(ts)}", kind: 'resolve', op: 'resolve.shell_detect',
                    api: 'Ukiryu::Shell.detect / Ukiryu::Shell.shell_class', platforms: UNIX, decision: 'D2',
                    input: { 'env' => { 'SHELL' => s, 'UKIRYU_TEST_SHELL' => ts } },
                    note: 'UKIRYU_TEST_SHELL override; invalid values are ignored by the Ruby oracle')
      end
      rows
    end

    # ------------------------------------------------------------------
    # lint / schema / yaml over every fixture and bundled schema

    def fixture_yaml
      base = File.join(Ops.corpus_dir, 'inputs')
      Dir[File.join(base, '{register,profiles,fixture-schema}', '**', '*.yaml')].sort
         .map { |f| f.delete_prefix("#{base}/") }
    end

    def schema_yaml
      Dir[File.join(Ops.corpus_dir, 'inputs', 'schemas', 'v1', '*.yaml')].sort
         .map { |f| f.delete_prefix(File.join(Ops.corpus_dir, 'inputs') + '/') }
    end

    def repo_yaml
      Dir[File.join(Ops.corpus_dir, 'inputs', 'repo-yaml', '**', '*.{yml,yaml}')].sort
         .map { |f| f.delete_prefix(File.join(Ops.corpus_dir, 'inputs') + '/') }
    end

    def fid(rel)
      slug(rel.sub(/\.ya?ml\z/, '').tr('/', '-'))
    end

    SYNTHETIC = [
      ['invalid-yaml', 'invalid: yaml: content: [unclosed'], ['scalar', 'just a string'], ['empty-doc', ''],
      ['minimal', "name: minimal\nversion: \"1.0\"\nprofiles:\n  - name: default\n    platforms: [linux]\n    shells: [bash]\n"],
      ['bad-names', "name: Bad Name\nversion: 1\nprofiles:\n  - name: default\n    commands:\n      - name: Do It\n"]
    ].freeze

    def lint_rows
      rows = fixture_yaml.map do |rel|
        row(id: "lint.fixture.#{fid(rel)}", kind: 'lint', op: 'lint.string',
            api: 'Ukiryu::Definition::DefinitionLinter.lint_string', input: { 'file' => rel }, sources: [rel])
      end
      rows + SYNTHETIC.map do |label, y|
        row(id: "lint.synthetic.#{label}", kind: 'lint', op: 'lint.string',
            api: 'Ukiryu::Definition::DefinitionLinter.lint_string', input: { 'yaml' => y })
      end
    end

    def bundled_schema_for(rel)
      case rel
      when %r{\Aregister/interfaces/} then 'schemas/v1/interface.schema.yaml'
      when %r{\Aregister/tools/[^/]+/index\.yaml\z} then 'schemas/v1/implementation_index.schema.yaml'
      when %r{\Aregister/tools/} then 'schemas/v1/implementation_version.schema.yaml'
      when %r{\Aprofiles/} then 'schemas/v1/tool.schema.yaml'
      end
    end

    def schema_rows
      rows = []
      api = 'Ukiryu::SchemaValidator.validate_profile'
      dapi = 'Ukiryu::Definition::DefinitionValidator.validate_string'
      fixture_yaml.each do |rel|
        next if rel.start_with?('fixture-schema/')

        rows << row(id: "schema.fixture-tool-schema.#{fid(rel)}", kind: 'schema', op: 'schema.validate_profile',
                    api: api, input: { 'file' => rel, 'schema' => 'fixture-schema/tool.schema.yaml' },
                    sources: [rel, 'fixture-schema/tool.schema.yaml'])
        bs = bundled_schema_for(rel)
        if bs
          rows << row(id: "schema.bundled.#{fid(rel)}", kind: 'schema', op: 'schema.validate_profile', api: api,
                      input: { 'file' => rel, 'schema' => bs }, sources: [rel, bs])
        end
        next unless rel.start_with?('profiles/') || rel.match?(%r{\Aregister/tools/[^/]+/[^/]+/})

        %w[fixture-schema/tool.schema.yaml schemas/v1/tool.schema.yaml].each do |sc|
          rows << row(id: "schema.definition-validator.#{fid(sc)}.#{fid(rel)}", kind: 'schema',
                      op: 'schema.definition_validate', api: dapi, input: { 'file' => rel, 'schema' => sc },
                      sources: [rel, sc])
        end
      end
      rows << row(id: 'schema.missing-path.unset', kind: 'schema', op: 'schema.validate_profile', api: api,
                  input: { 'file' => 'profiles/inkscape_1.0.yaml', 'schema' => nil },
                  sources: ['profiles/inkscape_1.0.yaml'], note: 'no schema_path and UKIRYU_SCHEMA_PATH unset')
      rows << row(id: 'schema.missing-path.nonexistent', kind: 'schema', op: 'schema.validate_profile', api: api,
                  input: { 'file' => 'profiles/inkscape_1.0.yaml', 'schema' => 'schemas/v1/missing.schema.yaml' },
                  sources: ['profiles/inkscape_1.0.yaml'])
      SYNTHETIC.each do |label, y|
        rows << row(id: "schema.synthetic.#{label}", kind: 'schema', op: 'schema.validate_profile', api: api,
                    input: { 'yaml' => y, 'schema' => 'fixture-schema/tool.schema.yaml' },
                    sources: ['fixture-schema/tool.schema.yaml'])
      end
      rows
    end

    def model_for(rel)
      case rel
      when %r{\Aregister/interfaces/} then 'Interface'
      when %r{\Aregister/tools/[^/]+/index\.yaml\z} then 'ImplementationIndex'
      when %r{\Aregister/tools/} then 'ImplementationVersion'
      end
    end

    def yaml_rows
      rows = (fixture_yaml + schema_yaml + repo_yaml).map do |rel|
        row(id: "yaml.load.#{fid(rel)}", kind: 'yaml', op: 'yaml.load',
            api: 'YAML.safe_load(permitted_classes: [Symbol, Date, Time]) (Psych)', input: { 'file' => rel },
            sources: [rel])
      end
      fixture_yaml.each do |rel|
        m = model_for(rel)
        if m
          rows << row(id: "yaml.model.#{fid(rel)}", kind: 'yaml', op: 'yaml.model_load',
                      api: "Ukiryu::Models::#{m}.from_yaml", input: { 'file' => rel, 'model' => m }, sources: [rel])
        elsif rel.start_with?('profiles/')
          rows << row(id: "yaml.roundtrip.#{fid(rel)}", kind: 'yaml', op: 'yaml.lutaml_roundtrip',
                      api: 'Ukiryu::Models::ToolDefinition.from_yaml / #to_yaml',
                      input: { 'file' => rel, 'model' => 'ToolDefinition' }, sources: [rel])
        end
      end
      rows
    end

    # ------------------------------------------------------------------
    # cli-output: golden outputs of the Ruby CLI against the fixture register

    SYN = '${INPUTS}/synthetic-register'
    CLI = [
      ['version', %w[version]], ['help', %w[help]], ['help-exec', %w[help exec]],
      ['commands-jq', %w[commands jq]], ['commands-unknown', %w[commands nosuchtool]],
      ['opts-jq', %w[opts jq]], ['opts-jq-process', %w[opts jq process]],
      ['describe-absent-text', ['describe', 'absenttool', '--register', SYN]],
      ['describe-absent-yaml', ['describe', 'absenttool', '--format', 'yaml', '--register', SYN]],
      ['describe-absent-json', ['describe', 'absenttool', '--format', 'json', '--register', SYN]],
      ['describe-absent-process', ['describe', 'absenttool', 'process', '--register', SYN]],
      ['info-absent', ['info', 'absenttool', '--register', SYN]],
      ['info-unknown', %w[info nosuchtool]],
      ['resolve-jq', %w[resolve jq]], ['resolve-jq-1-0', %w[resolve jq 1.0]],
      ['validate-file', ['validate', 'file', '${INPUTS}/profiles/inkscape_1.0.yaml',
                         '--schema', "${INPUTS}/#{PINNED_REMOTE_SCHEMA['path']}"]],
      ['lint-file', ['validate', 'lint', '${INPUTS}/profiles/inkscape_1.0.yaml']],
      ['exec-dry-run-jq', ['exec', 'jq', 'process', 'filter=.a', '--dry-run']],
      ['exec-dry-run-json', ['exec', 'jq', 'process', 'filter=.a', '--dry-run', '--format', 'json']],
      ['exec-dry-run-absent', ['exec', 'absenttool', 'process', 'input=a b.txt', 'level=3', '--dry-run',
                               '--register', SYN]],
      ['unknown-verb', %w[frobnicate]]
    ].freeze

    def cli_rows
      CLI.map do |label, argv|
        row(id: "cli-output.#{label}", kind: 'cli-output', op: 'cli.run', api: 'exe/ukiryu (Thor CLI)',
            input: { 'argv' => argv }, platforms: %w[linux], requires: %w[cli process fs],
            sources: label == 'validate-file' ? [PINNED_REMOTE_SCHEMA['path']] : [],
            note: 'captured on Linux only; stdout/stderr/exit compared byte-exact after ${INPUTS}/${HOME} substitution')
      end
    end

    # ------------------------------------------------------------------

    GENERATORS = %i[escape_rows validate_rows build_rows resolve_rows lint_rows schema_rows yaml_rows cli_rows].freeze

    def generate(out)
      ENV['UKIRYU_CONFORMANCE_DIR'] = out
      FileUtils.rm_rf(File.join(out, 'inputs'))
      FileUtils.rm_rf(File.join(out, 'cases'))
      inputs = copy_inputs(out)
      Ops::Sandbox.setup!
      rows = GENERATORS.flat_map { |g| send(g).tap { |rs| warn "generated #{rs.size} rows by #{g}" } }
      rows.each_with_index do |r, n|
        warn "executed #{n}/#{rows.size}" if (n % 100).zero?
        outcome = Runner.execute(r)
        r['expect'] = outcome
        r['sources'] = r['sources'].map do |s|
          { 'path' => s, 'sha256' => Digest::SHA256.file(File.join(out, 'inputs', s)).hexdigest }
        end
      end
      ids = rows.map { |r| r['id'] }
      dups = ids.tally.select { |_, c| c > 1 }.keys
      raise "duplicate generated ids: #{dups.first(10)}" unless dups.empty?

      FileUtils.mkdir_p(File.join(out, 'cases'))
      files = {}
      rows.group_by { |r| r['kind'] }.sort.each do |kind, krows|
        rel = "cases/#{kind}.jsonl"
        body = krows.sort_by { |r| r['id'] }.map { |r| "#{JSON.generate(order(r))}\n" }.join
        File.write(File.join(out, rel), body)
        files[rel] = Digest::SHA256.hexdigest(body)
      end
      applicable = Runner::PLATFORMS.to_h do |pf|
        [pf, rows.count { |r| Runner.applicable?(r, pf, Runner::CAPABILITIES) }]
      end
      manifest = {
        'corpus' => 'ukiryu-conformance', 'format_version' => 1,
        'oracle' => oracle_info,
        'cases' => { 'total' => rows.size, 'by_kind' => rows.map { |r| r['kind'] }.tally.sort.to_h,
                     'expect_error' => rows.count { |r| r['expect'].key?('error') },
                     'decision_pending' => rows.count { |r| r['decision'] },
                     'ids_sha256' => Runner.ids_digest(ids), 'files' => files },
        'applicable' => applicable,
        'inputs' => inputs
      }
      File.write(File.join(out, 'manifest.json'), "#{JSON.pretty_generate(manifest)}\n")
      manifest
    end

    def order(r)
      Runner::ROW_KEYS.select { |k| r.key?(k) }.to_h { |k| [k, r[k]] }
    end

    def oracle_info
      lock = File.join(__dir__, 'Gemfile.lock')
      gems = %w[ukiryu versionian lutaml-model lutaml-xsd json-schema thor psych paint].to_h do |g|
        [g, Gem.loaded_specs[g]&.version&.to_s]
      end
      {
        'implementation' => 'ukiryu (Ruby)', 'repository' => 'https://github.com/ukiryu/ukiryu',
        'commit' => source_commit, 'ruby' => RUBY_VERSION, 'psych' => Psych::VERSION,
        'libyaml' => Psych::LIBYAML_VERSION, 'captured_on' => Runner.host_platform,
        'gemfile_lock_sha256' => Digest::SHA256.file(lock).hexdigest, 'gems' => gems
      }
    end
  end
end

if $PROGRAM_NAME == __FILE__
  corpus = File.expand_path('../conformance', __dir__)
  if ARGV.include?('--check')
    Dir.mktmpdir('ukiryu-corpus-check') do |tmp|
      UkiryuOracle::Capture.generate(tmp)
      diff = `diff -r #{corpus} #{tmp} 2>&1`
      # README and schema are authored, not generated.
      diff = diff.lines.reject { |l| l =~ /\AOnly in #{Regexp.escape(corpus)}: (README\.md|case\.schema\.json)$/ }.join
      if diff.empty?
        puts 'capture --check: regenerated corpus is byte-identical'
      else
        warn diff.lines.first(40).join
        warn 'capture --check: regenerated corpus differs from committed corpus'
        exit 1
      end
    end
  else
    m = UkiryuOracle::Capture.generate(corpus)
    puts JSON.pretty_generate(m['cases'].except('files')), JSON.generate(m['applicable'])
  end
end
