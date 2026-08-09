package com.slot2sync.app

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.util.Base64
import androidx.activity.result.ActivityResult
import androidx.documentfile.provider.DocumentFile
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

// Lado Android do storage de saves do Slot2Sync.
//
// Implementa o contrato consumido pela MobileStorage em Rust
// (src-tauri/src/sync/mobile_storage.rs): comandos listFiles/stat/exists/read/
// write/copy sobre uma "árvore" concedida pelo Storage Access Framework (SAF).
// `tree` é a URI da árvore (content://...); `rel` é o caminho relativo a ela.

@InvokeArg
class ListArgs {
    lateinit var tree: String
    lateinit var base: String
}

@InvokeArg
class DocArgs {
    lateinit var tree: String
    lateinit var rel: String
}

@InvokeArg
class WriteArgs {
    lateinit var tree: String
    lateinit var rel: String
    lateinit var dataBase64: String
    var mtimeMs: Long? = null
}

@InvokeArg
class CopyArgs {
    lateinit var srcTree: String
    lateinit var srcRel: String
    lateinit var destTree: String
    lateinit var destRel: String
}

@TauriPlugin
class StoragePlugin(private val activity: Activity) : Plugin(activity) {
    private val ctx get() = activity.applicationContext

    private fun root(tree: String): DocumentFile? =
        DocumentFile.fromTreeUri(ctx, Uri.parse(tree))

    /** Resolve `tree`+`rel` num documento existente, ou null. */
    private fun resolve(tree: String, rel: String): DocumentFile? {
        var cur = root(tree) ?: return null
        for (seg in rel.split('/')) {
            if (seg.isEmpty()) continue
            cur = cur.findFile(seg) ?: return null
        }
        return cur
    }

    /** Cria as pastas-pai (se faltarem) e devolve o documento de arquivo final. */
    private fun ensureFile(tree: String, rel: String): DocumentFile? {
        var cur = root(tree) ?: return null
        val segs = rel.split('/').filter { it.isNotEmpty() }
        if (segs.isEmpty()) return null
        for (i in 0 until segs.size - 1) {
            cur = cur.findFile(segs[i])?.takeIf { it.isDirectory }
                ?: cur.createDirectory(segs[i]) ?: return null
        }
        val name = segs.last()
        return cur.findFile(name) ?: cur.createFile("application/octet-stream", name)
    }

    private fun walk(dir: DocumentFile, prefix: String, out: JSArray) {
        for (child in dir.listFiles()) {
            val name = child.name ?: continue
            val rel = if (prefix.isEmpty()) name else "$prefix/$name"
            if (child.isDirectory) {
                walk(child, rel, out)
            } else {
                val e = JSObject()
                e.put("rel", rel)
                e.put("mtimeMs", child.lastModified())
                e.put("size", child.length())
                out.put(e)
            }
        }
    }

    @Command
    fun listFiles(invoke: Invoke) {
        val a = invoke.parseArgs(ListArgs::class.java)
        val baseDir = resolve(a.tree, a.base)
        val entries = JSArray()
        if (baseDir != null && baseDir.isDirectory) {
            walk(baseDir, "", entries)
        }
        val ret = JSObject()
        ret.put("entries", entries)
        invoke.resolve(ret)
    }

    @Command
    fun stat(invoke: Invoke) {
        val a = invoke.parseArgs(DocArgs::class.java)
        val d = resolve(a.tree, a.rel) ?: return invoke.reject("não encontrado: ${a.rel}")
        val ret = JSObject()
        ret.put("mtimeMs", d.lastModified())
        invoke.resolve(ret)
    }

    @Command
    fun exists(invoke: Invoke) {
        val a = invoke.parseArgs(DocArgs::class.java)
        val ret = JSObject()
        ret.put("exists", resolve(a.tree, a.rel) != null)
        invoke.resolve(ret)
    }

    @Command
    fun read(invoke: Invoke) {
        val a = invoke.parseArgs(DocArgs::class.java)
        val d = resolve(a.tree, a.rel) ?: return invoke.reject("não encontrado: ${a.rel}")
        val bytes = ctx.contentResolver.openInputStream(d.uri)?.use { it.readBytes() }
            ?: return invoke.reject("falha ao abrir: ${a.rel}")
        val ret = JSObject()
        ret.put("dataBase64", Base64.encodeToString(bytes, Base64.NO_WRAP))
        invoke.resolve(ret)
    }

    @Command
    fun write(invoke: Invoke) {
        val a = invoke.parseArgs(WriteArgs::class.java)
        val bytes = Base64.decode(a.dataBase64, Base64.DEFAULT)
        val doc = ensureFile(a.tree, a.rel) ?: return invoke.reject("falha ao criar: ${a.rel}")
        ctx.contentResolver.openOutputStream(doc.uri, "wt")?.use { it.write(bytes) }
            ?: return invoke.reject("falha ao gravar: ${a.rel}")
        // SAF não expõe ajuste confiável de mtime; `mtimeMs` é ignorado (ver README).
        invoke.resolve(JSObject())
    }

    @Command
    fun copy(invoke: Invoke) {
        val a = invoke.parseArgs(CopyArgs::class.java)
        val src = resolve(a.srcTree, a.srcRel) ?: return invoke.reject("origem não encontrada")
        val bytes = ctx.contentResolver.openInputStream(src.uri)?.use { it.readBytes() }
            ?: return invoke.reject("falha ao ler origem")
        val dst = ensureFile(a.destTree, a.destRel) ?: return invoke.reject("falha ao criar destino")
        ctx.contentResolver.openOutputStream(dst.uri, "wt")?.use { it.write(bytes) }
            ?: return invoke.reject("falha ao gravar destino")
        invoke.resolve(JSObject())
    }

    /**
     * Abre o seletor de pasta do SO (SAF). A árvore concedida é devolvida em
     * `{ tree }` — o Rust a guarda como `root_path` do emulador. A permissão é
     * persistida para sobreviver a reinícios do app.
     */
    @Command
    fun pickFolder(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
        startActivityForResult(invoke, intent, "onFolderPicked")
    }

    @ActivityCallback
    fun onFolderPicked(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            return invoke.reject("seleção de pasta cancelada")
        }
        val flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or
            Intent.FLAG_GRANT_WRITE_URI_PERMISSION
        ctx.contentResolver.takePersistableUriPermission(uri, flags)
        val ret = JSObject()
        ret.put("tree", uri.toString())
        invoke.resolve(ret)
    }
}
