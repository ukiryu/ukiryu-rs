# frozen_string_literal: true
# Fixture preparation only. Uses the same locked, pinned oracle as the corpus.
require 'bundler/setup'
require 'ukiryu'
require 'json'
require 'digest'
require 'open3'
abort 'unexpected oracle' unless Gem.loaded_specs.fetch('ukiryu').version.to_s == '0.3.1'
source = Gem.loaded_specs.fetch('ukiryu').full_gem_path
commit, status = Open3.capture2('git', '-C', source, 'rev-parse', 'HEAD')
abort 'cannot read oracle commit' unless status.success?
commit = commit.strip
abort 'unexpected Ruby' unless RUBY_VERSION == '4.0.1'
lock_hash = Digest::SHA256.file(ENV.fetch('BUNDLE_GEMFILE') + '.lock').hexdigest
abort 'unexpected oracle lock' unless lock_hash == 'fc3b12f2ede259028b638d2f26c65ddf9506d142c20a8bf0c1b151741078b65e'
abort "unexpected commit #{commit}" unless commit == 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
random = Random.new(230019)
atoms = ["", "a", " ", "\t", "\n", "\r", "\0", "'", '"', '`', '$', '\\', '!', ';', '&', '|', '(', ')', '[', ']', '{', '}', '-', '=', 'Ω', '💎', "\u2028", "\u0085", 'C:\\Program Files\\tool.exe', '-Command', '-File', '$env:HOME']
strings = atoms + Array.new(512) { Array.new(random.rand(1..24)) { atoms.sample(random: random) }.join }
rows = []
%w[powershell tcsh].each do |name|
  shell = Ukiryu::Shell.const_get(name == 'powershell' ? 'PowerShell' : 'Tcsh').new
  strings.each_with_index do |string, i|
    operations = { 'escape' => { 'string' => string }, 'quote' => { 'string' => string }, 'needs_quoting' => { 'string' => string }, 'join' => { 'executable' => string, 'args' => [strings[(i + 1) % strings.size], '-Command', strings[(i + 7) % strings.size]] } }
    if name == 'powershell'
      operations['escape_for_double_quotes'] = { 'string' => string }
      operations['quote_for_exe_true'] = { 'string' => string, 'for_exe' => true }
      operations['quote_for_exe_false'] = { 'string' => string, 'for_exe' => false }
    end
    operations.each do |label, input|
      method = label.start_with?('quote_for_exe') ? 'quote' : label
      value = case method
              when 'join' then shell.join(input.fetch('executable'), *input.fetch('args'))
              when 'needs_quoting' then shell.needs_quoting?(string)
              when 'quote' then input.key?('for_exe') ? shell.quote(string, for_exe: input['for_exe']) : shell.quote(string)
              else shell.public_send(method, string)
              end
      rows << { id: "wire-extra.#{name}.#{i}.#{label}", request: { op: 'escape', shell: name, operation: { method: method }.merge(input) }, value: value }
    end
  end
end
output = File.join(__dir__, 'supplemental.jsonl')
File.write(output, rows.map { |row| JSON.generate(row) }.join("\n") + "\n")
provenance = { oracle_commit: commit, ruby: RUBY_VERSION, seed: 230019, strings_per_shell: strings.size, rows: rows.size, sha256: Digest::SHA256.file(output).hexdigest, gemfile_lock_sha256: Digest::SHA256.file(ENV.fetch('BUNDLE_GEMFILE') + '.lock').hexdigest, source_sha256: %w[base powershell tcsh].to_h { |name| path = File.join(source, 'lib/ukiryu/shell', name + '.rb'); [name, Digest::SHA256.file(path).hexdigest] } }
File.write(File.join(__dir__, 'provenance.json'), JSON.pretty_generate(provenance) + "\n")
puts JSON.generate(provenance)
