package chat.gnobbi.mobile

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.matrix_mobile.MobileException
import uniffi.matrix_mobile.MobileSession
import uniffi.matrix_mobile.RoomItem
import uniffi.matrix_mobile.SecretStore
import uniffi.matrix_mobile.SyncListener
import uniffi.matrix_mobile.SyncState
import uniffi.matrix_mobile.listSavedAccounts
import uniffi.matrix_mobile.restoreSession
import uniffi.matrix_mobile.signIn

/** Per-account database key: random bytes, sealed with an Android Keystore AES key, kept in preferences. */
class KeystoreSecrets(context: Context) : SecretStore {
    private val prefs = context.getSharedPreferences("store-keys", Context.MODE_PRIVATE)

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
            generateKey()
        }
    }

    override fun create(account: ULong): String = try {
        val secret = Base64.encodeToString(ByteArray(32).also { SecureRandom().nextBytes(it) }, Base64.NO_WRAP)
        val cipher = Cipher.getInstance(TRANSFORM).apply { init(Cipher.ENCRYPT_MODE, key()) }
        val sealed = cipher.iv + cipher.doFinal(secret.toByteArray())
        prefs.edit().putString(account.toString(), Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()
        secret
    } catch (e: Exception) { throw MobileException.SecretStore() }

    override fun read(account: ULong): String = try {
        val sealed = Base64.decode(prefs.getString(account.toString(), null)!!, Base64.NO_WRAP)
        val cipher = Cipher.getInstance(TRANSFORM).apply { init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, sealed, 0, 12)) }
        String(cipher.doFinal(sealed, 12, sealed.size - 12))
    } catch (e: Exception) { throw MobileException.SecretStore() }

    override fun delete(account: ULong) {
        prefs.edit().remove(account.toString()).commit()
    }

    private companion object {
        const val ALIAS = "gnobbi-store"
        const val TRANSFORM = "AES/GCM/NoPadding"
    }
}

/** Bridge callbacks come from the Rust events thread; Compose state is snapshot-safe to write from there. */
class Session(context: Context) : SyncListener {
    var rooms by mutableStateOf<List<RoomItem>>(emptyList())
    var state by mutableStateOf(SyncState.CONNECTING)
    var lastSend by mutableStateOf("")
    var error by mutableStateOf("")
    var signedIn by mutableStateOf(false)
    private var handle: MobileSession? = null
    private val secrets = KeystoreSecrets(context)
    private val dir = // Created by the bridge (mode 0700); it must be the canonical path: /data/user/0 is a symlink to /data/data.
    context.filesDir.canonicalFile.resolve("gnobbi").path

    override fun onStatus(state: SyncState) { this.state = state }
    override fun onRooms(rooms: List<RoomItem>) { this.rooms = rooms }
    override fun onSendResult(roomId: String, transactionId: String, error: MobileException?) {
        lastSend = if (error == null) "Messaggio inviato" else "Invio non riuscito: $error"
    }

    suspend fun signIn(homeserver: String, user: String, password: String, ca: String?) =
        open { signIn(dir, homeserver, user, password, secrets, this, ca) }

    suspend fun restore(ca: String?) = open {
        // First saved account only: no account picker yet.
        val account = listSavedAccounts(dir).firstOrNull() ?: throw MobileException.NoSession()
        restoreSession(dir, account, secrets, this, ca)
    }

    private suspend fun open(make: suspend () -> MobileSession) {
        error = ""
        try { handle = make(); signedIn = true }
        catch (e: MobileException.NoSession) { /* first launch: stay on the sign-in form */ }
        catch (e: MobileException) { error = "Accesso non riuscito: $e" }
    }

    fun send(room: RoomItem, text: String) {
        lastSend = try { handle?.sendText(room.roomId, text); "Invio in corso…" }
        catch (e: MobileException) { "Invio non riuscito: $e" }
    }
}
