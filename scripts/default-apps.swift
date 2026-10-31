// Sets the default app per file type, with no Finder pop-up.
// Run by step 71-default-apps (rust/agent-config/src/dots/install/default_apps.rs). That step
// reads default-apps.list, checks it, and sends the result here on stdin: one "bundle<TAB>uti"
// per line. Usage: swift default-apps.swift < pairs.tsv
//
// Why not the public APIs: NSWorkspace.setDefaultApplication shows one pop-up per type, and
// LSSetDefaultRoleHandlerForContentType returns 0 but changes nothing on current macOS.
// What works: write LSHandlers in the Launch Services preferences, then restart lsd so it
// reloads them. This is the same file the Finder dialog writes.
//
// Prints one line: "changed N" or "ok N". Exit 1 when a change did not read back.
// A line "skip ..." on stderr is not an error (app not installed, type unknown).

import AppKit
import CoreServices
import Foundation
import UniformTypeIdentifiers

func say(_ s: String) { FileHandle.standardError.write(Data((s + "\n").utf8)) }

func current(_ uti: String) -> String? {
    LSCopyDefaultRoleHandlerForContentType(uti as CFString, LSRolesMask.all)?.takeRetainedValue() as String?
}

let input = String(data: FileHandle.standardInput.readDataToEndOfFile(), encoding: .utf8) ?? ""

// (uti, bundle id) pairs that need a change.
var wanted: [(String, String)] = []
var installed: [String: Bool] = [:]
var total = 0
for raw in input.components(separatedBy: .newlines) {
    let parts = raw.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
    if parts.count != 2 || parts[0].isEmpty || parts[1].isEmpty { continue }
    let (bundle, uti) = (parts[0], parts[1])
    if installed[bundle] == nil {
        let ok = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) != nil
        installed[bundle] = ok
        if !ok { say("skip \(bundle): not installed") }
    }
    if installed[bundle] != true { continue }
    if UTType(uti) == nil {
        say("skip \(uti): unknown type")
        continue
    }
    total += 1
    if current(uti)?.lowercased() != bundle.lowercased() { wanted.append((uti, bundle)) }
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
if !CFPreferencesSynchronize(domain, kCFPreferencesCurrentUser, kCFPreferencesAnyHost) {
    say("could not write the Launch Services preferences")
    exit(1)
}

// lsd caches the handlers; a restart makes it read the file again (launchd starts it back).
let kill = Process()
kill.executableURL = URL(fileURLWithPath: "/usr/bin/killall")
kill.arguments = ["lsd"]
kill.standardError = FileHandle.nullDevice
do {
    try kill.run()
    kill.waitUntilExit()
} catch {
    say("could not restart lsd: \(error.localizedDescription)")
}

// Read every change back, for up to 10 seconds.
var bad: [(String, String)] = wanted
for _ in 0..<20 {
    bad = bad.filter { current($0.0)?.lowercased() != $0.1.lowercased() }
    if bad.isEmpty { break }
    Thread.sleep(forTimeInterval: 0.5)
}
if !bad.isEmpty {
    for (uti, bid) in bad { say("not applied: \(uti) -> \(bid)") }
    exit(1)
}
print("changed \(wanted.count)")
