# Código nativo mobile — plugin de storage

Lado nativo do plugin de armazenamento de saves, consumido pela `MobileStorage`
em Rust (`src-tauri/src/sync/mobile_storage.rs`). Ver o contrato dos comandos e o
panorama em [Status multiplataforma](https://jidvdij.github.io/slot2sync-site/docs/referencia/status-multiplataforma/).

## Android — `android/StoragePlugin.kt`

Implementa `pickFolder` (concessão via `ACTION_OPEN_DOCUMENT_TREE`) e
`listFiles`/`stat`/`exists`/`read`/`write`/`copy` sobre o Storage Access Framework
(SAF), usando a URI da árvore concedida (`tree`) + caminho relativo (`rel`).

### Onde colocar

O Rust registra o plugin com
`api.register_android_plugin("com.slot2sync.app", "StoragePlugin")`
(em `mobile_storage::init()`), então a classe precisa estar no classpath do app,
no pacote `com.slot2sync.app`. Depois de `npm run tauri android init`, copie para:

```
src-tauri/gen/android/app/src/main/java/com/slot2sync/app/StoragePlugin.kt
```

### Dependência Gradle

Adicione ao `dependencies` do módulo `app` (em `gen/android/app/build.gradle.kts`):

```kotlin
implementation("androidx.documentfile:documentfile:1.0.1")
```

### Caveats conhecidos (resolver na validação em device)

- **mtime não ajustável:** o SAF não permite definir o `lastModified` de um
  documento. A convergência de mtime que o `SyncEngine` usa no desktop (mtime local
  = `modifiedTime` do Drive) não vale aqui — avaliar guardar o mtime do Drive num
  sidecar/manifest, ou tratar o mobile como "uma origem só de escrita confiável" no
  diff.
- **Nome de arquivo no `createFile`:** o provedor pode anexar extensão conforme o
  mime. Saves têm nomes exatos — testar; se necessário, criar com mime mais
  específico ou renomear após criar.
- **`Android/data/` bloqueado:** o seletor de árvore (Android 11+) não concede
  acesso a `Android/data/`; depende de o emulador guardar saves em shared storage
  (memstick configurável do PPSSPP).

## iOS — pendente

`mobile_storage::init()` tem o registro iOS como `todo!()`. Implementar:

- `register_ios_plugin` no Rust + um Swift package com a classe do plugin expondo os
  mesmos comandos via `UIDocumentPickerViewController` + security-scoped bookmark
  (`startAccessingSecurityScopedResource`). Fazer e validar no macOS/Xcode.

## Fluxo de concessão de pasta (UI) — falta o wiring Rust/TS

O `pickFolder` nativo (Android) já está no esqueleto. Falta:

- um comando Tauri (`#[cfg(mobile)]`, em `commands.rs`) que invoque `pickFolder` via a
  ponte e devolva a `tree` ao frontend;
- o wrapper TS (`src/lib/ipc.ts`) + a UI de "conceder pasta do emulador";
- persistir a árvore concedida como `root_path` do emulador (no mobile o `root_path`
  guarda a URI da pasta concedida — ver doc de `mobile_storage`).
