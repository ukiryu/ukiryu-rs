# frozen_string_literal: true

require 'json'
require 'time'

module UkiryuOracle
  # Typed JSON encoding for values that cross the corpus boundary.
  #
  # JSON null/true/false/integers/strings/arrays map to themselves. Ruby
  # values that JSON cannot carry losslessly are tagged objects:
  #
  #   Symbol             {"$sym": "name"}
  #   Float              {"$float": "<Float#to_s>"}  (exact decimal text)
  #   Time               {"$time": "<iso8601 with 9 fraction digits>"}
  #   Regexp             {"$regexp": "<source>"}
  #   Hash, string keys  JSON object (key order not significant)
  #   Hash, other keys   {"$map": [[key, value], ...]}  (order significant)
  #
  # A plain JSON object whose only key starts with "$" is reserved for tags.
  module Codec
    module_function

    def encode(value)
      case value
      when nil, true, false, Integer then value
      when String then value.encoding == Encoding::BINARY ? { '$bytes' => value.unpack1('H*') } : value
      when Symbol then { '$sym' => value.to_s }
      when Float then { '$float' => value.to_s }
      when Time then { '$time' => value.utc.iso8601(9) }
      when Regexp then { '$regexp' => value.source }
      when Array then value.map { |v| encode(v) }
      when Hash
        if value.keys.all?(String)
          value.to_h { |k, v| [k, encode(v)] }
        else
          { '$map' => value.map { |k, v| [encode(k), encode(v)] } }
        end
      else
        raise ArgumentError, "unencodable value of class #{value.class}"
      end
    end

    def decode(value)
      case value
      when Array then value.map { |v| decode(v) }
      when Hash
        if value.size == 1 && value.keys.first.start_with?('$')
          tag, payload = value.first
          case tag
          when '$sym' then payload.to_sym
          when '$float' then Float(payload)
          when '$time' then Time.iso8601(payload)
          when '$regexp' then Regexp.new(payload)
          when '$bytes' then [payload].pack('H*')
          when '$map' then payload.to_h { |k, v| [decode(k), decode(v)] }
          else raise ArgumentError, "unknown tag #{tag}"
          end
        else
          value.transform_values { |v| decode(v) }
        end
      else value
      end
    end

    # Structural equality on encoded values: objects compare key-order
    # insensitively, arrays and $map entries in order, strings byte-exact.
    def structural_equal?(a, b)
      case a
      when Hash
        return false unless b.is_a?(Hash) && a.keys.sort == b.keys.sort

        a.all? { |k, v| structural_equal?(v, b[k]) }
      when Array
        b.is_a?(Array) && a.size == b.size && a.zip(b).all? { |x, y| structural_equal?(x, y) }
      when String
        b.is_a?(String) && a.b == b.b
      else
        a == b && a.class == b.class
      end
    end

    # Deterministic JSON text: keys sorted recursively.
    def canonical_json(value)
      JSON.generate(sort_keys(value))
    end

    def sort_keys(value)
      case value
      when Hash then value.keys.sort.to_h { |k| [k, sort_keys(value[k])] }
      when Array then value.map { |v| sort_keys(v) }
      else value
      end
    end
  end
end
