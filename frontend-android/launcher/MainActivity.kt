package dev.dioxus.main

import android.content.ActivityNotFoundException
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Bundle
import android.provider.MediaStore
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.FileProvider
import java.io.File

typealias BuildConfig = eu.matmoa.bouedig.BuildConfig

class MainActivity : WryActivity() {
    private lateinit var libraryLauncher: ActivityResultLauncher<Intent>
    private lateinit var cameraLauncher: ActivityResultLauncher<Intent>
    private var lastCaptureUri: Uri? = null

    // The import screen polls for these files from Rust (same uid): a photo
    // means "here it is", the cancel marker means "user backed out".
    private fun importFile() = File(cacheDir, "import.jpg")
    private fun cancelFile() = File(cacheDir, "import.cancelled")

    // Launchers must be registered while the activity is only CREATED —
    // the webview (and onWebViewCreate) arrives later on the looper.
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        libraryLauncher = registerForActivityResult(
            ActivityResultContracts.StartActivityForResult()
        ) { result -> finishPick(result.data?.data) }
        cameraLauncher = registerForActivityResult(
            ActivityResultContracts.StartActivityForResult()
        ) { _ -> finishPick(lastCaptureUri) }
    }

    override fun onWebViewCreate(webView: WebView) {
        // dx ships RustWebChromeClient.kt but the webview's own
        // <input type=file> never opens a chooser on Android (wry gap), so
        // the import screen picks through this native bridge instead: the
        // usual photo library / camera apps, downscaled, then written to the
        // app cache where the Rust side finds it.
        webView.addJavascriptInterface(BouedigBridge(), "BouedigNative")
    }

    /// Downscales the picked/captured image into the app cache (the largest
    /// edge ≤ 1600 px, JPEG q85 — plenty for extraction) and writes the done
    /// marker for the Rust poll.
    private fun finishPick(uri: Uri?) {
        if (uri == null) {
            cancelFile().createNewFile()
            return
        }
        try {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            val first = contentResolver.openInputStream(uri)
            if (first == null) {
                cancelFile().createNewFile()
                return
            }
            first.use {
                BitmapFactory.decodeStream(it, null, bounds)
            }
            val sample = maxOf(1, maxOf(bounds.outWidth, bounds.outHeight) / 1600)
            val options = BitmapFactory.Options().apply { inSampleSize = sample }
            val bitmap = contentResolver.openInputStream(uri)?.use {
                BitmapFactory.decodeStream(it, null, options)
            }
            if (bitmap == null) {
                cancelFile().createNewFile()
                return
            }
            bitmap.compress(Bitmap.CompressFormat.JPEG, 85, importFile().outputStream())
        } catch (ex: Exception) {
            Logger.error("native image pick failed: " + ex.message)
            cancelFile().createNewFile()
        }
    }

    /// Exposed to the import screen as window.BouedigNative.
    inner class BouedigBridge {
        @JavascriptInterface
        fun pickFromLibrary() {
            val picker = Intent(Intent.ACTION_GET_CONTENT)
            picker.addCategory(Intent.CATEGORY_OPENABLE)
            picker.type = "image/*"
            try {
                libraryLauncher.launch(picker)
            } catch (ex: ActivityNotFoundException) {
                Logger.warn(Logger.tags("Bridge"), "no activity for image picking")
                cancelFile().createNewFile()
            }
        }

        @JavascriptInterface
        fun takePhoto() {
            val photoFile = try {
                createCaptureFile()
            } catch (ex: Exception) {
                Logger.error("camera file failed: " + ex.message)
                return
            }
            val uri = FileProvider.getUriForFile(
                this@MainActivity,
                packageName + ".fileprovider",
                photoFile,
            )
            lastCaptureUri = uri
            val capture = Intent(MediaStore.ACTION_IMAGE_CAPTURE)
            capture.putExtra(MediaStore.EXTRA_OUTPUT, uri)
            capture.addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_READ_URI_PERMISSION)
            try {
                cameraLauncher.launch(capture)
            } catch (ex: ActivityNotFoundException) {
                Logger.warn(Logger.tags("Bridge"), "no camera app")
                cancelFile().createNewFile()
            }
        }
    }

    @Throws(Exception::class)
    private fun createCaptureFile(): File {
        val timeStamp = java.text.SimpleDateFormat("yyyyMMdd_HHmmss")
            .format(java.util.Date())
        val storageDir = getExternalFilesDir(android.os.Environment.DIRECTORY_PICTURES)
            ?: cacheDir
        return File.createTempFile("JPEG_" + timeStamp + "_", ".jpg", storageDir)
    }
}