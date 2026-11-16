# frozen_string_literal: true

# Oracle capture for the platform-family discovery layouts and the bundled
# definition search (issue #18, UK18-A3 follow-up).
#
# Runs the ORIGINAL pinned Ruby code (ukiryu ab6df353) against directory trees
# built here and writes JSON golden data to stdout. Nothing is installed;
# every tree is created under a fresh temp directory.
#
#   ruby -I<oracle>/lib capture_layouts.rb [<scratch-root>] > layouts.json
#
# What is real: ExecutableLocator.find_in_path (PathScanner, PATHEXT) and the
# two Tool class methods bundled_definition_search_paths and from_bundled,
# with the real Dir.glob, File.expand_path and Platform.detect. The two
# methods are cut out of lib/ukiryu/tool.rb by line range and evaluated
# unchanged, because requiring the whole gem needs the lutaml-model stack.
# What is stubbed: Tool.load only. The stand-in reads the file, raises
# DefinitionLoadError for a file that starts with "invalid", and otherwise
# returns the "version:" value. The stub does not change which files
# from_bundled tries or in what order.
#
# The discovery cases use only names and modes that every host agrees on, so
# the same golden is checked natively on Linux, macOS and Windows:
# PATHEXT always lists ".exe" first, "executable" files are mode 0755 and end
# in .exe or .cmd, the non-executable file is mode 0644 and ends in .txt, and
# unix_only cases (a bare-name executable) are skipped on Windows hosts.

require 'json'
require 'fileutils'
require 'rbconfig'
require 'tmpdir'
require 'ukiryu/platform'
require 'ukiryu/errors'
require 'ukiryu/executable_locator'
require 'ukiryu/tool_finder'

ORACLE_SHA = 'ab6df353c6dc2d7faf7817a86aafcafa82579896'
ROOT = File.realpath(Dir.mktmpdir('ukiryu-layouts', ARGV[0]))

def rel(path)
  path&.gsub(ROOT, '<ROOT>')
end

def abs(path)
  path.gsub('<ROOT>', ROOT)
end

def build(root, layout)
  layout.each do |entry|
    path = File.join(root, entry['path'])
    FileUtils.mkdir_p(File.dirname(path))
    case entry['kind']
    when 'file'
      File.write(path, entry['content'] || "#!/bin/sh\nexit 0\n")
      File.chmod(entry['mode'], path)
    when 'dir'
      FileUtils.mkdir_p(path)
    else
      raise "unknown kind #{entry['kind']}"
    end
  end
end

def with_env(dirs, pathext)
  old_path = ENV['PATH']
  old_ext = ENV['PATHEXT']
  ENV['PATH'] = dirs.map { |d| abs(d) }.join(':')
  pathext.nil? ? ENV.delete('PATHEXT') : ENV['PATHEXT'] = pathext
  Ukiryu::Platform.reset_cache
  yield
ensure
  ENV['PATH'] = old_path
  old_ext.nil? ? ENV.delete('PATHEXT') : ENV['PATHEXT'] = old_ext
  Ukiryu::Platform.reset_cache
end

# ---- 1. platform layouts: executable discovery ----

EXE = "#!/bin/sh\nexit 0\n"
layout = [
  # macOS bundle
  { 'path' => 'Applications/Foo.app/Contents/MacOS/foo', 'kind' => 'file', 'mode' => 0o755 },
  { 'path' => 'Applications/Foo.app/Contents/Resources/readme.txt', 'kind' => 'file', 'mode' => 0o644 },
  # Windows Program Files
  { 'path' => 'Program Files/Tool/bin/tool.exe', 'kind' => 'file', 'mode' => 0o755 },
  { 'path' => 'Program Files (x86)/Legacy/bin/legacy.exe', 'kind' => 'file', 'mode' => 0o755 },
  { 'path' => 'Program Files (x86)/Tool/bin/tool.exe', 'kind' => 'file', 'mode' => 0o755 },
  # spaces in several directory names
  { 'path' => 'My Programs/Sub Dir/Tool Home/tool.exe', 'kind' => 'file', 'mode' => 0o755 },
  # a directory that shadows the executable name, then the real file
  { 'path' => 'shadow/tool.exe/inner.txt', 'kind' => 'file', 'mode' => 0o644 },
  { 'path' => 'real/tool.exe', 'kind' => 'file', 'mode' => 0o755 },
  # non-executable file with the only extension tried first
  { 'path' => 'noexec/tool.txt', 'kind' => 'file', 'mode' => 0o644 },
  # PATHEXT order: tool.exe before tool.cmd in the same directory
  { 'path' => 'both/tool.cmd', 'kind' => 'file', 'mode' => 0o755 },
  { 'path' => 'both/tool.exe', 'kind' => 'file', 'mode' => 0o755 },
  { 'path' => 'cmdonly/tool.cmd', 'kind' => 'file', 'mode' => 0o755 },
  { 'path' => 'empty', 'kind' => 'dir' }
]

PF = '<ROOT>/Program Files/Tool/bin'
PF86 = '<ROOT>/Program Files (x86)/Tool/bin'
MAC = '<ROOT>/Applications/Foo.app/Contents/MacOS'
EXT = '.exe;.cmd'

# name, path dirs, pathext, command, additional dirs, unix_only
cases = [
  ['macos-bundle-dir-on-path', [MAC], nil, 'foo', [], true],
  ['macos-bundle-dir-in-additional-paths', ['<ROOT>/empty'], nil, 'foo', [MAC], true],
  ['macos-applications-dir-on-path-not-probed', ['<ROOT>/Applications'], nil, 'foo', [], false],
  ['macos-app-root-on-path-not-probed', ['<ROOT>/Applications/Foo.app'], nil, 'foo', [], false],
  ['macos-contents-dir-on-path-not-probed', ['<ROOT>/Applications/Foo.app/Contents'], nil, 'foo', [], false],
  ['macos-bundle-name-is-not-a-command', ['<ROOT>/Applications'], nil, 'Foo', [], false],
  ['windows-program-files-on-path', [PF], EXT, 'tool', [], false],
  ['windows-program-files-in-additional-paths', ['<ROOT>/empty'], EXT, 'tool', [PF], false],
  ['windows-program-files-x86-in-additional-paths', ['<ROOT>/empty'], EXT, 'legacy',
   ['<ROOT>/Program Files (x86)/Legacy/bin'], false],
  ['windows-program-files-first-on-path-wins', [PF, PF86], EXT, 'tool', [], false],
  ['windows-program-files-x86-first-on-path-wins', [PF86, PF], EXT, 'tool', [], false],
  ['windows-program-files-not-searched-unless-listed', ['<ROOT>/empty'], EXT, 'tool', [], false],
  ['windows-program-files-parent-not-probed', ['<ROOT>/Program Files'], EXT, 'tool', [], false],
  ['windows-program-files-bin-parent-not-probed', ['<ROOT>/Program Files/Tool'], EXT, 'tool', [], false],
  ['spaces-in-directory-names', ['<ROOT>/My Programs/Sub Dir/Tool Home'], EXT, 'tool', [], false],
  ['spaces-with-trailing-slash', ['<ROOT>/My Programs/Sub Dir/Tool Home/'], EXT, 'tool', [], false],
  ['directory-shadowing-exe-is-skipped', ['<ROOT>/shadow', '<ROOT>/real'], EXT, 'tool', [], false],
  ['directory-shadowing-exe-alone-is-absent', ['<ROOT>/shadow'], EXT, 'tool', [], false],
  ['non-executable-file-is-skipped', ['<ROOT>/noexec', '<ROOT>/real'], '.txt;.exe', 'tool', [], false],
  ['non-executable-file-alone-is-absent', ['<ROOT>/noexec'], '.txt;.exe', 'tool', [], false],
  ['pathext-exe-before-cmd', ['<ROOT>/both'], EXT, 'tool', [], false],
  ['pathext-falls-to-cmd', ['<ROOT>/cmdonly'], EXT, 'tool', [], false],
  ['pathext-only-exe-ignores-cmd', ['<ROOT>/cmdonly'], '.exe', 'tool', [], false],
  ['pathext-dir-order-before-extension-order', ['<ROOT>/cmdonly', '<ROOT>/both'], EXT, 'tool', [], false]
]

build(ROOT, layout)

discovery = cases.map do |name, dirs, pathext, command, additional, unix_only|
  found = nil
  with_env(dirs, pathext) do
    found = Ukiryu::ExecutableLocator.find_in_path(command, additional_paths: additional.map { |d| abs(d) })
  end
  { 'name' => name, 'path' => dirs, 'pathext' => pathext, 'command' => command,
    'additional' => additional, 'unix_only' => unix_only, 'found' => rel(found) }
end

# ---- 2. bundled definition search paths ----

tool_source = File.readlines(File.join(File.dirname($LOADED_FEATURES.grep(%r{ukiryu/platform\.rb\z}).first), 'tool.rb'))
# from_bundled through bundled_definition_search_paths (pinned lines 716-757).
raise 'tool.rb moved' unless tool_source[715].include?('def from_bundled') &&
                             tool_source[733].include?('def bundled_definition_search_paths')

Ukiryu.const_set(:Tool, Class.new)
Ukiryu::Tool.singleton_class.class_eval(tool_source[715..756].join)

def search_paths_for(platform, home)
  old_home = ENV['HOME']
  ENV['HOME'] = home
  sc = Ukiryu::Platform.singleton_class
  sc.alias_method(:__orig_detect, :detect)
  sc.define_method(:detect) { platform }
  Ukiryu::Tool.bundled_definition_search_paths
ensure
  sc.alias_method(:detect, :__orig_detect)
  sc.remove_method(:__orig_detect)
  ENV['HOME'] = old_home
end

HOME = '/home/tester'
# File.expand_path('C:/...') is relative to the working directory on a
# non-Windows host, so the directory it ran from is written as <CWD> and the
# Rust test strips that prefix.
CWD = Dir.pwd
search_paths = %i[linux macos windows].map do |platform|
  { 'platform' => platform.to_s, 'home' => HOME,
    'host_expand_prefix' => '<CWD>/',
    'paths' => search_paths_for(platform, HOME).map { |p| p.sub(CWD, '<CWD>') } }
end

# ---- 3. from_bundled over rooted trees ----

# Stand-in for Tool.load (see header). Not oracle code.
class << Ukiryu::Tool
  attr_accessor :loaded

  def load(file_path, _options = {})
    text = File.read(file_path)
    raise Ukiryu::Errors::DefinitionLoadError, "invalid #{file_path}" if text.start_with?('invalid')

    loaded << file_path
    text[/^version: '?([^'\n]*)'?/, 1]
  end
end

def good(version)
  "name: foo\nversion: '#{version}'\n"
end

BAD = "invalid: [\n"
SYS = 'usr/share/ukiryu'
LOCAL = 'usr/local/share/ukiryu'
BREW = 'opt/homebrew/share/ukiryu'
USER = 'home/tester/.local/share/ukiryu'

def f(path, content)
  { 'path' => path, 'kind' => 'file', 'mode' => 0o644, 'content' => content }
end

# name, files, tool
bundled_cases = [
  ['first-directory-wins', [f("#{SYS}/foo/1.yaml", good('sys')), f("#{LOCAL}/foo/1.yaml", good('local'))], 'foo'],
  ['falls-to-later-directory', [f("#{LOCAL}/foo/1.yaml", good('local'))], 'foo'],
  ['homebrew-directory', [f("#{BREW}/foo/1.yaml", good('brew'))], 'foo'],
  ['user-local-directory-last', [f("#{USER}/foo/1.yaml", good('user')), f("#{BREW}/foo/1.yaml", good('brew'))], 'foo'],
  ['user-local-directory-when-alone', [f("#{USER}/foo/1.yaml", good('user'))], 'foo'],
  ['files-in-sorted-order', [f("#{SYS}/foo/b.yaml", good('b')), f("#{SYS}/foo/a.yaml", good('a')),
                            f("#{SYS}/foo/10.yaml", good('10')), f("#{SYS}/foo/2.yaml", good('2'))], 'foo'],
  ['invalid-file-skipped-same-directory', [f("#{SYS}/foo/a.yaml", BAD), f("#{SYS}/foo/b.yaml", good('b'))], 'foo'],
  ['invalid-file-skipped-next-directory', [f("#{SYS}/foo/a.yaml", BAD), f("#{LOCAL}/foo/a.yaml", good('local'))], 'foo'],
  ['all-invalid-is-nil', [f("#{SYS}/foo/a.yaml", BAD), f("#{LOCAL}/foo/a.yaml", BAD)], 'foo'],
  ['other-extensions-ignored', [f("#{SYS}/foo/1.yml", good('yml')), f("#{SYS}/foo/1.yaml.bak", good('bak')),
                                f("#{SYS}/foo/YAML", good('x'))], 'foo'],
  ['hidden-file-ignored', [f("#{SYS}/foo/.hidden.yaml", good('hidden'))], 'foo'],
  ['nested-directory-not-searched', [f("#{SYS}/foo/sub/1.yaml", good('nested'))], 'foo'],
  ['other-tool-directory-not-searched', [f("#{SYS}/bar/1.yaml", good('bar'))], 'foo'],
  ['tool-name-is-a-directory-prefix-only', [f("#{SYS}/foobar/1.yaml", good('foobar'))], 'foo'],
  ['spaces-in-tool-name', [f("#{SYS}/my tool/1.yaml", good('spaced'))], 'my tool'],
  ['no-directories-at-all', [], 'foo']
]

ORIGINAL_SEARCH_PATHS = Ukiryu::Tool.method(:bundled_definition_search_paths)

bundled = bundled_cases.map do |name, files, tool|
  root = File.join(ROOT, 'bundled', name)
  FileUtils.mkdir_p(root)
  build(root, files)
  old_home = ENV['HOME']
  ENV['HOME'] = HOME
  sc = Ukiryu::Platform.singleton_class
  sc.alias_method(:__orig_detect, :detect)
  sc.define_method(:detect) { :linux }
  Ukiryu::Tool.define_singleton_method(:bundled_definition_search_paths) { ORIGINAL_SEARCH_PATHS.call.map { |p| root + p } }
  Ukiryu::Tool.loaded = []
  version = Ukiryu::Tool.from_bundled(tool)
  { 'name' => name, 'tool' => tool, 'files' => files.map { |e| e.slice('path', 'content') },
    'version' => version, 'tried' => Ukiryu::Tool.loaded.map { |p| p.delete_prefix("#{root}/") } }
ensure
  Ukiryu::Tool.define_singleton_method(:bundled_definition_search_paths, ORIGINAL_SEARCH_PATHS)
  sc.alias_method(:detect, :__orig_detect)
  sc.remove_method(:__orig_detect)
  ENV['HOME'] = old_home
end

out = {
  'provenance' => {
    'oracle' => 'ukiryu/ukiryu',
    'sha' => ORACLE_SHA,
    'ruby' => RUBY_VERSION,
    'host_os' => RbConfig::CONFIG['host_os'],
    'files' => %w[lib/ukiryu/executable_locator.rb lib/ukiryu/platform.rb lib/ukiryu/tool.rb],
    'script' => 'ukiryu/tests/discovery_layouts_data/capture/capture_layouts.rb',
    'stubs' => 'Tool.load only (reads the file; "invalid" first line raises DefinitionLoadError; else returns the version: value); Platform.detect and HOME for the search-path cases'
  },
  'discovery_layout' => layout,
  'discovery' => discovery,
  'search_paths' => search_paths,
  'bundled' => bundled
}
puts JSON.pretty_generate(out)
FileUtils.rm_rf(ROOT)
