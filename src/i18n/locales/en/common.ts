export const common = {
  common: {
    close: "Close",
    add: "Add",
    dismiss: "Dismiss",
  },
  app: {
    checkingConnection: "Checking remote provider connection…",
    settings: "⚙ Settings",
    emulators: "Emulators",
    loading: "loading…",
    noEmulators:
      "No emulators configured yet. Use “Add emulator” and select the PPSSPP or PCSX2 root folder.",
    switchToLightTheme: "☀ Light theme",
    switchToDarkTheme: "🌙 Dark theme",
  },
  update: {
    title: "Version {{version}} available",
    install: "Update and restart",
    installing: "Updating…",
    later: "Not now",
  },
  panic: {
    title: "Something failed unexpectedly",
    body: "An internal operation crashed. The app is still running, but the action that triggered it may not have completed. Details are in the log file.",
    at: "at",
  },
} as const;
