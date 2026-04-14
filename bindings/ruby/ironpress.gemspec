# frozen_string_literal: true

Gem::Specification.new do |spec|
  spec.name          = "ironpress"
  spec.version       = "1.4.1"
  spec.authors       = ["Paul Gaston Gouron"]
  spec.summary       = "Pure Rust HTML/CSS/Markdown to PDF converter"
  spec.description   = "Convert HTML, CSS, and Markdown to PDF with no browser or system dependencies. Native Rust extension for maximum performance."
  spec.homepage      = "https://github.com/gastongouron/ironpress"
  spec.license       = "MIT"
  spec.required_ruby_version = ">= 3.0"

  spec.files         = Dir["lib/**/*.rb", "ext/**/*", "Cargo.toml", "src/**/*.rs", "README.md"]
  spec.extensions    = ["ext/ironpress/extconf.rb"]
  spec.require_paths = ["lib"]

  spec.add_dependency "rb_sys", "~> 0.9"
end
