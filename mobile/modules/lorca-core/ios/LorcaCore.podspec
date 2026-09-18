require 'json'

package = JSON.parse(File.read(File.join(__dir__, '..', 'package.json')))

# The Rust core built by build.ts: a static library for the device and the simulator in one
# xcframework, and the UniFFI Swift bindings beside it.
Pod::Spec.new do |s|
  s.name           = 'LorcaCore'
  s.version        = package['version']
  s.summary        = package['description']
  s.description    = package['description']
  s.license        = 'MIT'
  s.author         = 'Lorca'
  s.homepage       = 'https://lorca.app'
  s.platforms      = { :ios => '16.4' }
  s.swift_version  = '5.9'
  s.source         = { git: '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  # The xcframework carries the C header and its module map; the Swift bindings import it.
  s.source_files = "*.swift", "generated/*.swift"
  s.vendored_frameworks = 'LorcaCore.xcframework'
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'SWIFT_COMPILATION_MODE' => 'wholemodule',
  }
end
