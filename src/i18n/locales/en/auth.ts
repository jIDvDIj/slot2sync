export const auth = {
  login: {
    tagline: "Keep your emulator saves and savestates in sync across devices.",
    providerLabel: "Where to keep your saves",
    googleDriveHint: "Stored in a Slot2Sync folder in your Drive",
    cloudHint: "Stored in a Slot2Sync folder in your account",
    localFolderHint: "A folder on this computer or on a network share",
    comingSoon: "Coming Soon",
    folderPathLabel: "Folder",
    folderPathPlaceholder: "e.g. D:\\Slot2Sync or \\\\server\\share",
    chooseFolder: "Choose Folder…",
    folderPickerTitle: "Choose where Slot2Sync keeps your saves",
    deviceHint: "Shown next to every save, so you can tell which device changed it.",
    permissionNote:
      "Slot2Sync <strong>can't see your personal files</strong>. It only reads and changes the files it creates.",
    continueWith: "Continue with {{provider}}",
    connectFolder: "Connect Folder",
    waitingForBrowser: "Waiting for you to authorize in your browser…",
    connectingFolder: "Connecting to the folder…",
  },
  device: {
    nameLabel: "This device's name",
    namePlaceholder: "e.g. Gaming PC, Laptop",
  },
  account: {
    connected: "Connected",
    disconnect: "Disconnect",
  },
} as const;
