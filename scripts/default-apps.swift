// Sets the default app per file type from default-apps.list, with no Finder pop-up.
// Run it through scripts/default-apps.sh. Usage: swift default-apps.swift <list-file>
//
// Why not the public APIs: NSWorkspace.setDefaultApplication shows one pop-up per type, and
// LSSetDefaultRoleHandlerForContentType returns 0 but changes nothing on current macOS.
// What works: write LSHandlers in the Launch Services preferences, then restart lsd so it
// reloads them. This is the same file the Finder dialog writes.
//
// Prints one line: "changed N" or "ok N". Exit 1 when a change did not read back.

import AppKit
import CoreServices
import Foundation

guard CommandLine.arguments.count > 1,
      let text = try? String(contentsOfFile: CommandLine.arguments[1], encoding: .utf8) else {
    FileHandle.standardError.write(Data("usage: default-apps.swift <list-file>\n".utf8))
    exit(2)
}

func current(_ uti: String) -> String? {
    LSCopyDefaultRoleHandlerForContentType(uti as CFString, LSRolesMask.all)?.takeRetainedValue() as String?
}

// (uti, bundle id) pairs that need a change.
var wanted: [(String, String)] = []
var bundle = ""
var installed = false
var total = 0
for raw in text.split(separator: "\n") {
    let line = raw.trimmingCharacters(in: .whitespaces)
    if line.isEmpty || line.hasPrefix("#") { continue }
    if line.hasPrefix("@") {
        bundle = String(line.dropFirst())
        installed = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) != nil
        if !installed { FileHandle.standardError.write(Data("skip \(bundle): not installed\n".utf8)) }
        continue
    }
    if bundle.isEmpty || !installed { continue }
    total += 1
    if current(line)?.lowercased() != bundle.lowercased() { wanted.append((line, bundle)) }
}

if wanted.isEmpty {
    print("ok \(total)")
    exit(0)
}

let domain = "com.apple.LaunchServices/com.apple.launchservices.secure" as CFString
let key = "LSHandlerContentType"
var handlers = (CFPreferencesCopyAppValue("LSHandlers" as CFString, domain) as? [[String: Any]]) ?? []
for (uti, bid) in wanted {
    if let i = handlers.firstIndex(where: { ($0[key] as? String)?.lowercased() == uti.lowercased() }) {
        handlers[i]["LSHandlerRoleAll"] = bid
        handlers[i]["LSHandlerPreferredVersions"] = ["LSHandlerRoleAll": "-"]
    } else {
        handlers.append([key: uti, "LSHandlerRoleAll": bid, "LSHandlerPreferredVersions": ["LSHandlerRoleAll": "-"]])
    }
}
CFPreferencesSetValue("LSHandlers" as CFString, handlers as CFArray, domain, kCFPreferencesCurrentUser, kCFPreferencesAnyHost)
CFPreferencesSynchronize(domain, kCFPreferencesCurrentUser, kCFPreferencesAnyHost)

// lsd caches the handlers; a restart makes it read the file again (launchd starts it back).
let kill = Process()
kill.executableURL = URL(fileURLWithPath: "/usr/bin/killall")
kill.arguments = ["lsd"]
kill.standardError = FileHandle.nullDevice
try? kill.run()
kill.waitUntilExit()

// Read every change back, for up to 10 seconds.
var bad: [(String, String)] = wanted
for _ in 0..<20 {
    bad = bad.filter { current($0.0)?.lowercased() != $0.1.lowercased() }
    if bad.isEmpty { break }
    Thread.sleep(forTimeInterval: 0.5)
}
if !bad.isEmpty {
    for (uti, bid) in bad { FileHandle.standardError.write(Data("not applied: \(uti) -> \(bid)\n".utf8)) }
    exit(1)
}
print("changed \(wanted.count)")
