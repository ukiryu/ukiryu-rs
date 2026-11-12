# frozen_string_literal: true
# Regenerate (Ruby 4.0.1, bundle of the pinned checkout's Gemfile.lock):
#   UKIRYU_RUBY_SRC=<ukiryu checkout at ab6df353> bundle exec ruby -I$UKIRYU_RUBY_SRC/lib capture_regex.rb > regex.json
# Oracle capture: Ruby (Onigmo) semantics of version-detection patterns as the
# pinned source uses them: `String#match(String)` (implicit Regexp.new) and
# `MatchData#[1]`. Emits golden JSON consumed by ukiryu/tests/version_detection_regex.rs.
require 'json'
require 'yaml'

# Checkout of ukiryu/ukiryu at ab6df353c6dc2d7faf7817a86aafcafa82579896.
SRC = ENV.fetch('UKIRYU_RUBY_SRC')

# 1. Inventory every version-detection pattern in the pinned fixtures.
fixture_patterns = []
Dir.glob("#{SRC}/spec/fixtures/**/*.yaml").sort.each do |path|
  data = YAML.safe_load(File.read(path), aliases: true) rescue next
  rel = path.sub("#{SRC}/", '')
  walk = lambda do |node, trail|
    case node
    when Hash
      node.each do |k, v|
        if %w[version_detection detection].include?(k.to_s) && v.is_a?(Hash) && v.key?('pattern')
          fixture_patterns << { 'file' => rel, 'key' => (trail + [k.to_s, 'pattern']).join('.'), 'pattern' => v['pattern'] }
        end
        if k.to_s == 'version_detection' && v.is_a?(Hash) && v['detection_methods'].is_a?(Array)
          v['detection_methods'].each_with_index do |m, i|
            next unless m.is_a?(Hash) && m.key?('pattern')
            fixture_patterns << { 'file' => rel, 'key' => "#{k}.detection_methods.#{i}.pattern", 'pattern' => m['pattern'] }
          end
        end
        walk.call(v, trail + [k.to_s])
      end
    when Array
      node.each_with_index { |v, i| walk.call(v, trail + [i.to_s]) }
    end
  end
  walk.call(data, [])
end
fixture_patterns.reject! { |p| p['file'].end_with?('tool.schema.yaml') }

FIXTURE_INPUTS = [
  "ImageMagick 7.1.1-29\nVersion: ImageMagick 7.1.1-29 Q16-HDRI x86_64 https://imagemagick.org",
  "Version: ImageMagick 6.9.11-60 Q16 x86_64 2021-01-25 https://imagemagick.org",
  "gzip 1.12\nCopyright (C) 2018 Free Software Foundation, Inc.",
  "bzip2, a block-sorting file compressor.  Version 1.0.8, 13-Jul-2019.",
  "UnZip 6.00 of 20 April 2009, by Debian. Original by Info-ZIP.",
  "jq-1.7.1", "yq (https://github.com/mikefarah/yq/) version v4.40.5", "yq 3.4.3",
  "GPL Ghostscript 10.02.1 (2023-11-01)", "10.02.1",
  "Inkscape 1.2.2 (b0a8486541, 2022-12-01)", "Inkscape 0.92.5 (2060ec1f9f, 2020-04-08)",
  "ffmpeg version 6.1.1-3ubuntu5 Copyright (c) 2000-2023 the FFmpeg developers",
  "ping from iputils 20240117", "ping from iputils s20161105",
  "PING localhost (127.0.0.1) 56(84) bytes of data.", "Pinging 127.0.0.1 with 32 bytes of data:",
  "sort (GNU coreutils) 9.4", "tar (GNU tar) 1.35", "bsdtar 3.7.2 - libarchive 3.7.2",
  "exiftool 12.76", "pdf2ps 10.02", "BusyBox v1.36.1 (2023-11-07) multi-call binary.",
  "", "no version here", "Version: GraphicsMagick 1.3", "version 7", "1.2"
].freeze

# 2. Boundary cases: [pattern, input] where Ruby and Rust regex can disagree.
BOUNDARY = [
  ['(\d+\.\d+)', "v١٢.٣"],            # Arabic-Indic digits: \d is ASCII in Ruby
  ['(\d+\.\d+)', "v１.２ and 3.4"],           # fullwidth digits then ASCII
  ['([\d.]+)', "٣.٤ 9.8"],
  ['x(\w+)', "xété"],                      # \w ASCII-only?
  ['x([\w.-]*)', "x1.2-é"],
  ['a(\s)b', "a b"],                             # NBSP not \s
  ['a(\s)b', "a\vb"],                                 # \v is \s since 2.2
  ['a(\s)b', "a\fb"],
  ['a(\s)b', "a b"],
  ['(\h+)', "zz١ 0xBEEFg"],                      # \h hex digit
  ['(\H+)', "abcXYZ"],
  ['(\D+)', "١٢x"],
  ['(\W+)', "ééa"],
  ['(\S+)', " x y"],
  ['^(\d+)$', "abc\n42\nxyz"],                       # ^/$ are line anchors in Ruby
  ['^(\d+)', "x\n7"],
  ['(\d+)$', "9\nx"],
  ['(.)$', "ab\n"],
  ['\A(\d+)', "x\n7"],
  ['(\d+)\z', "7\n"],
  ['(\d+)\Z', "7\n"],
  ['x$', "x\r\n"],
  ['^$', "a\n\nb"],
  ['(^)', "a\n"],
  ['\n(^)', "a\n"],
  ['a(.)c', "a\nc"],                                  # . excludes \n
  ['a(.)c', "aéc"],
  ['a(.)c', "a\rc"],
  ['(?m)a(.)c', "a\nc"],                              # Ruby (?m) is dotall
  ['(?i)version (\d+)', "VERSION 3"],
  ['(?i:ver)sion (\d+)', "VERsion 3"],
  ['(?i)(é+)', "Éé"],
  ['(?i)(k)', "K"],                              # Kelvin sign case fold
  ['(?<maj>\d+)\.(\d+)', "1.2"],                      # named + plain group: plain not captured
  ['(?<maj>\d+)\.(?<min>\d+)', "1.2"],
  ["(?'maj'\\d+)", "5"],
  ['(?:v)(\d+)', "v9"],
  ['(a)|(b)', "b"],                                   # group 1 unmatched -> nil
  ['(a)?b', "b"],
  ['(\d+)', "x"],
  ['PING', "PING x"],                                 # no capture group
  ['', "anything"],
  ['x{2}(\d)', "xx1"],
  ['x{,2}(\d)', "xx1"],                               # {,n} valid in Ruby
  ['x{2,}(\d)', "xxx1"],
  ['x{a}(\d)', "x{a}1"],                              # invalid interval is literal
  ['x{2,1}', "xx"],
  ['x{', "x{"],
  ['(\d+?)', "123"],
  ['(\d*+)1', "111"],                                 # possessive
  ['(\d++)', "111"],
  ['(?=v)(v\d)', "v1"],                                # lookahead
  ['(?<=v)(\d)', "v1"],                                # lookbehind
  ['(?!x)(\d)', "1"],
  ['(?<!x)(\d)', "1"],
  ['(?>a+)(b)', "aab"],                               # atomic
  ['(a)\1', "aa"],                                    # backref
  ['(?<n>a)\k<n>', "aa"],
  ['\p{Alpha}(\d)', "a1"],
  ['[[:alpha:]](\d)', "é1"],
  ['[[:digit:]]+', "١"],
  ['[a-z&&[^aeiou]](\d)', "b1"],
  ['[a[0-9]]+', "a1"],
  ['[]a](\d)', "]1"],
  ['[^]a](\d)', "x1"],
  ['[a-](\d)', "-1"],
  ['[-a](\d)', "-1"],
  ['[\-](\d)', "-1"],
  ['[\]](\d)', "]1"],
  ['[\\\\](\d)', "\\1"],
  ['[\d-x](\d)', "-1"],
  ['[\w-](\d)', "-1"],
  ['[.](\d)', "a1 .2"],
  ['[\t ](\d)', " 1"],
  ['[^\d](\d)', "a1"],
  ['[^\D](\d)', "11"],
  ['[\s\S](\d)', "x1"],
  ['[\h](\d)', "f1"],
  ['[~](\d)', "~1"],
  ['[a~~b](\d)', "~1"],
  ['[a--b](\d)', "-1"],
  ['[&](\d)', "&1"],
  ['[a&b](\d)', "&1"],
  ['\x41(\d)', "A1"],
  ['é(\d)', "é1"],
  ['\t(\d)', "\t1"],
  ['\.(\d)', ".1"],
  ['\/(\d)', "/1"],
  ['\-(\d)', "-1"],
  ['\#(\d)', "#1"],
  ['\ (\d)', " 1"],
  ['\y(\d)', "y1"],
  ['\e(\d)', "\e1"],
  ['\a(\d)', "\a1"],
  ['\0(\d)', "\u00001"],
  ['\bv(\d)', "xv1 v2"],
  ['a\b', "aé"],
  ['\Ba', "éa"],
  ['\Gv(\d)', "v1"],
  ['v\K(\d)', "v1"],
  ['\R(\d)', "\r\n1"],
  ['\X(\d)', "e1"],
  ['(?x) v (\d) ', "v1"],
  ['(?#c)v(\d)', "v1"],
  ['(?i-m)v(\d)', "V1"],
  ['(?~abc)', "x"],
  ['(?u)\w', "é"],
  ['(?a)\w', "é"],
  ['#(\d)', "#1"],
  [' (\d)', " 1"],
  ['a|', "b"],
  ['|a', "b"],
  ['(', "("],
  [')', ")"],
  ['[a', "a"],
  ['*a', "a"],
  ['a**', "aa"],
  ['a{1}{2}', "aa"],
  ['a{1}?', "a"],
  ['\\', "x"],
  ['[]', "x"],
  ['[^]', "x"],
  ['(?<1a>x)', "x"],
  ['(?<n>x)(?<n>y)', "xy"],
  ['(?P<n>x)', "x"],
  ['\8', "8"],
  ['[z-a]', "a"],
  ['a{100000}', "a"],
  ['\cA', "\u0001"],
  ['\o{101}', "A"],
  ['\101(\d)', "A1"],
  ['(*a)', "a"],
  ['a|*b', "b"],
  ['(+a)', "a"],
  ['a(?)b', "ab"],
  ['x{,}', "x{,}"],
  ['{2}a', "{2}a"],
  ['^*a', "a"],
  ['^(\d*)$', "x\n"],
  ['(\d*)$', "x\n"],
  ['^', "a\n"],
  ['\n^', "a\n"],
  ['\n^', "a\nb"],
  ['(?i)ss', "ß"],
  ['(?i)(\w)', "K"],
  ['(?i)([\w])', "ſ"],
  ['(?i)([a-z])', "K"],
  ['x{1,2}?(\d)', "xx1"],
  ['x+?(\d)', "xx1"],
  ['x{1}+', "xx"],
  ['(?<n_1>v)(\d)', "v1"],
  ['(?<é>v)', "v"],
  ['(?<n>v)|(w)', "w"],
  ['[\x41](\d)', "A1"],
  ['\x4', "\u0004"],
  ['￿', "￿"],
  ['\u{41}', "A"],
].freeze

def compile(pat)
  re = Regexp.new(pat)
  { 'ok' => true, 'names' => re.names }
rescue RegexpError, ArgumentError => e
  { 'ok' => false, 'error_class' => e.class.name, 'message' => e.message }
end

def run_match(pat, input)
  m = input.match(pat)
  return nil unless m
  { 'm0' => m[0], 'm1' => m[1], 'begin0' => m.begin(0), 'size' => m.size }
end

def row(pat, input, origin)
  c = compile(pat)
  r = { 'origin' => origin, 'pattern' => pat, 'input' => input, 'compile' => c }
  if c['ok']
    r['match'] = run_match(pat, input)
    r['match_p'] = input.match?(Regexp.new(pat))
  end
  r
end

rows = []
fixture_patterns.map { |p| p['pattern'] }.uniq.each do |pat|
  next if pat.nil?
  FIXTURE_INPUTS.each { |inp| rows << row(pat, inp, 'fixture') }
end
BOUNDARY.each { |pat, inp| rows << row(pat, inp, 'boundary') }
# Default Regexp literal used by source: /(\d+\.\d+)/
default_re = /(\d+\.\d+)/
default_rows = FIXTURE_INPUTS.map do |inp|
  m = inp.match(default_re)
  { 'input' => inp, 'match' => m && { 'm0' => m[0], 'm1' => m[1], 'begin0' => m.begin(0), 'size' => m.size } }
end

out = {
  'provenance' => {
    'oracle' => 'ukiryu/ukiryu ab6df353c6dc2d7faf7817a86aafcafa82579896',
    'ruby' => RUBY_VERSION, 'onigmo' => (defined?(Regexp::ONIGMO_VERSION) ? Regexp::ONIGMO_VERSION : nil),
    'semantics' => 'String#match(String) / MatchData#[1] as in lib/ukiryu/version_detector.rb detect_info, lib/ukiryu/tool/loader.rb extract_version_from_pattern, lib/ukiryu/tools/base.rb detect_version',
    'script' => 'capture_regex.rb'
  },
  'fixture_patterns' => fixture_patterns,
  'default_literal' => { 'source' => default_re.source, 'rows' => default_rows },
  'rows' => rows
}
puts JSON.pretty_generate(out)
