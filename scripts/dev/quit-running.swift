#!/usr/bin/env swift
import AppKit
import Foundation

// Leave the shared router socket free for this checkout. Include bare Cargo
// launches, which have an executable name but no PwrRoute bundle identifier.
let running = NSWorkspace.shared.runningApplications.filter {
  $0.bundleIdentifier == "com.pwrdrvr.pwrroute"
    || $0.executableURL?.lastPathComponent == "pwrroute"
}
for app in running {
  print("Quitting PwrRoute (pid \(app.processIdentifier))")
  app.terminate()
}
let deadline = Date().addingTimeInterval(15)
while running.contains(where: { !$0.isTerminated }) && Date() < deadline {
  RunLoop.current.run(until: Date().addingTimeInterval(0.1))
}
if running.contains(where: { !$0.isTerminated }) {
  FileHandle.standardError.write(Data("PwrRoute did not quit; close it and press Start again.\n".utf8))
  exit(1)
}
