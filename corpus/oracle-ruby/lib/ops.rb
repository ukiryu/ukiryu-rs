# frozen_string_literal: true

require 'open3'
require 'rbconfig'
require 'tmpdir'
require 'yaml'
require_relative 'codec'
require 'ukiryu'
# Loaded lazily (require_relative inside methods) by the gem itself.
require 'ukiryu/version_scheme_resolver'

module UkiryuOracle
  # Operations executed against the pinned Ruby implementation.
  #
  # Each op takes the decoded `input` of a corpus row and returns a Ruby value
  # (encoded later by Codec) or raises. Ops call the real public or module
  # entrypoints of the pinned gem; nothing here re-implements Ukiryu logic.
  # Where an entrypoint is private in Ruby (`send`), the row's `api` field
  # names it so other engines know which behaviour is pinned.
  module Ops
    SHELLS = {
      'bash' => 'Bash', 'zsh' => 'Zsh', 'fish' => 'Fish', 'sh' => 'Sh', 'dash' => 'Dash',
      'tcsh' => 'Tcsh', 'powershell' => 'PowerShell', 'cmd' => 'Cmd'
    }.freeze

    module_function

    def corpus_dir
      ENV.fetch('UKIRYU_CONFORMANCE_DIR')
    end

    def input_path(rel)
      File.join(corpus_dir, 'inputs', rel)
    end

    # Replace host-specific absolute paths with stable tokens.
    def scrub(value)
      case value
      when String
        value.gsub(File.join(corpus_dir, 'inputs'), '${INPUTS}').gsub(Sandbox.home.to_s, '${HOME}')
      when Array then value.map { |v| scrub(v) }
      when Hash then value.to_h { |k, v| [scrub(k), scrub(v)] }
      else value
      end
    end

    # Shell diagnostics embed the host Ruby platform. Canonicalize only its
    # exact runtime value in the verified diagnostic field; a wrong value must
    # remain visible and fail byte comparison.
    def scrub_error(error, op:)
      message = scrub(error.message)
      return message unless op == 'resolve.shell_detect' && error.is_a?(Ukiryu::Errors::UnknownShellError)

      message.sub("\nCurrent environment:\n  Platform: #{RUBY_PLATFORM}\n",
                  "\nCurrent environment:\n  Platform: ${RUBY_PLATFORM}\n")
    end

    # Process-wide state every op starts from.
    module Sandbox
      class << self
        attr_reader :home

        def setup!
          @home ||= Dir.mktmpdir('ukiryu-oracle-home')
          ENV['HOME'] = @home
          ENV['TZ'] = 'UTC'
          ENV['LANG'] = 'C.UTF-8'
          ENV['LC_ALL'] = 'C.UTF-8'
          ENV['SHELL'] = '/bin/bash'
          ENV['UKIRYU_REGISTER'] = File.join(Ops.corpus_dir, 'inputs', 'register')
          %w[UKIRYU_SCHEMA_PATH UKIRYU_TEST_SHELL UKIRYU_DEBUG UKIRYU_DEBUG_EXECUTABLE UKIRYU_CONFIG
             UKIRYU_TIMEOUT UKIRYU_SHELL UKIRYU_FORMAT UKIRYU_LOG_LEVEL NO_COLOR
             PSModulePath MSYSTEM MINGW_PREFIX WSL_DISTRO].each { |k| ENV.delete(k) }
        end

        def reset!
          setup!
          Ukiryu::Config.reset!
          Ukiryu::Register.reset_default
          Ukiryu::ToolIndex.reset
          Ukiryu::Tool.clear_cache
          Ukiryu::Runtime.instance.reset!
          Ukiryu::Shell.reset
        end
      end
    end

    def run(op, input)
      Sandbox.reset!
      send("op_#{op.tr('.', '_')}", input)
    end

    # --- escape -----------------------------------------------------------

    def shell_instance(name)
      Ukiryu::Shell.const_get(SHELLS.fetch(name)).new
    end

    def op_shell_escape(i) = shell_instance(i['shell']).escape(i['string'])
    def op_shell_quote(i) = shell_instance(i['shell']).quote(i['string'])
    def op_shell_join(i) = shell_instance(i['shell']).join(i['executable'], *i['args'])
    def op_shell_env_var(i) = shell_instance(i['shell']).env_var(i['name'])
    def op_shell_format_path(i) = shell_instance(i['shell']).format_path(i['path'])

    # --- validate ---------------------------------------------------------

    def op_type_validate(i)
      Ukiryu::Type.validate(i['value'], i['type'], i['options'] || {})
    end

    # --- build ------------------------------------------------------------

    # Tool::Loader.load_with_implementation_index with the implementation and
    # version file given instead of detected from a running executable. Each
    # step returning nil ends the chain the way Tool.get does: with
    # ToolNotFoundError "Tool not found: <name>".
    # The builder's contract excludes host executable discovery. Preserve the
    # real Tool initializer and command builder, supplying a declared absent
    # executable instead of running where/which during every pure argv row.
    class BuilderTool < Ukiryu::Tool
      private

      def find_executable = nil
    end
    private_constant :BuilderTool

    def load_tool(i)
      opts = { platform: i['platform'].to_sym, shell: i['shell'].to_sym }
      not_found = -> { raise Ukiryu::Errors::ToolNotFoundError, "Tool not found: #{i['tool']}" }
      index = Ukiryu::Register.load_implementation_index(i['tool'], opts) || not_found.call
      interface = Ukiryu::Register.load_interface(index.interface, opts) || not_found.call
      impl_version = Ukiryu::Register.load_implementation_version(i['tool'], i['implementation'], i['file'], opts) ||
                     not_found.call
      tool_def = Ukiryu::Tool::Loader.send(:convert_to_tool_definition, i['tool'], interface, impl_version,
                                           i['implementation'], nil, opts) || not_found.call
      BuilderTool.new(tool_def, opts)
    end

    def op_build_args(i)
      tool = load_tool(i)
      command = tool.command_definition(i['command'])
      raise ArgumentError, "Unknown command: #{i['command']}" unless command

      params = (i['params'] || {}).transform_keys(&:to_sym)
      tool.build_args(command, params)
    end

    # --- resolve ----------------------------------------------------------

    def op_resolve_extract_version(i)
      Ukiryu::Tool::Loader.send(:extract_version_from_pattern, i['output'], i['pattern'])
    end

    def op_resolve_version_spec(i)
      index = Ukiryu::Register.load_implementation_index(i['tool'], {})
      impl = index.implementations.find { |m| (m[:name] || m['name']) == i['implementation'] }
      raise "no implementation #{i['implementation']}" unless impl

      scheme = Ukiryu::VersionSchemeResolver.resolve(impl[:version_scheme] || impl['version_scheme'])
      spec = Ukiryu::Tool::Loader.send(:find_matching_version_spec, impl[:versions] || impl['versions'],
                                       i['version'], scheme)
      spec && (spec[:file] || spec['file'])
    end

    def op_resolve_range(i)
      require 'versionian'
      scheme = Ukiryu::VersionSchemeResolver.resolve(i['scheme'])
      bounds = i['bounds'].transform_keys(&:to_sym)
      Versionian::VersionRange.new(i['range'].to_sym, scheme, **bounds).matches?(i['version'])
    end

    def op_resolve_shell_detect(i)
      i['env'].each { |k, v| v.nil? ? ENV.delete(k) : ENV[k] = v }
      Ukiryu::Shell.reset
      detected = Ukiryu::Shell.detect
      Ukiryu::Shell.reset
      klass = begin
        Ukiryu::Shell.shell_class.name
      rescue Ukiryu::Errors::UnknownShellError => e
        { 'error' => e.class.name }
      end
      { 'detect' => detected, 'shell_class' => klass }
    end

    # --- lint / schema ----------------------------------------------------

    def read_input(i)
      i.key?('yaml') ? i['yaml'] : File.read(input_path(i['file']))
    end

    def op_lint_string(i)
      Ukiryu::Definition::DefinitionLinter.lint_string(read_input(i)).to_h
    end

    def op_schema_validate_profile(i)
      profile = YAML.safe_load(read_input(i), permitted_classes: [Symbol, Date, Time])
      path = i['schema'] && input_path(i['schema'])
      Ukiryu::SchemaValidator.validate_profile(profile, schema_path: path)
    end

    def op_schema_definition_validate(i)
      Ukiryu::Definition::DefinitionValidator.validate_string(read_input(i),
                                                              schema_path: input_path(i['schema'])).to_h
    end

    # --- yaml -------------------------------------------------------------

    def op_yaml_load(i)
      YAML.safe_load(File.read(input_path(i['file'])), permitted_classes: [Symbol, Date, Time])
    end

    def op_yaml_lutaml_roundtrip(i)
      model = Ukiryu::Models.const_get(i['model'])
      first = model.from_yaml(File.read(input_path(i['file'])))
      emitted = first.to_yaml
      { 'emitted' => emitted, 'reparsed_equal' => model.from_yaml(emitted).to_yaml == emitted }
    end

    def op_yaml_model_load(i)
      model = Ukiryu::Models.const_get(i['model'])
      snapshot(model.from_yaml(input_path(i['file'])))
    end

    # Attribute snapshot of a plain Ruby model: class name plus instance
    # variables, recursively. Records presence/default behaviour as loaded.
    def snapshot(obj)
      case obj
      when nil, true, false, Integer, Float, String, Symbol then obj
      when Array then obj.map { |v| snapshot(v) }
      when Hash then obj.to_h { |k, v| [k, snapshot(v)] }
      else
        ivars = obj.instance_variables.sort.to_h { |iv| [iv.to_s, snapshot(obj.instance_variable_get(iv))] }
        { 'class' => obj.class.name, 'ivars' => ivars }
      end
    end

    # --- cli-output -------------------------------------------------------

    def op_cli_run(i)
      exe = File.join(Gem.loaded_specs.fetch('ukiryu').full_gem_path, 'exe', 'ukiryu')
      env = {
        'HOME' => Sandbox.home, 'TZ' => 'UTC', 'LANG' => 'C.UTF-8', 'LC_ALL' => 'C.UTF-8', 'SHELL' => '/bin/bash',
        'UKIRYU_REGISTER' => File.join(corpus_dir, 'inputs', 'register'),
        'COLUMNS' => '100', 'THOR_COLUMNS' => '100', 'TERM' => 'dumb',
        'GIT_REDIRECT_STDERR' => '/dev/null'
      }.merge(i['env'] || {})
      args = i['argv'].map { |a| a.gsub('${INPUTS}', File.join(corpus_dir, 'inputs')) }
      out, err, status = Open3.capture3(env, RbConfig.ruby, '-rbundler/setup', exe, *args,
                                        chdir: Sandbox.home, stdin_data: i['stdin'] || '')
      scrub({ 'stdout' => out, 'stderr' => err, 'exit' => status.exitstatus })
    end
  end
end
