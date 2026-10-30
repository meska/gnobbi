package chat.gnobbi.mobile

import android.os.Bundle
import android.util.Base64
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.gnobbi.design.DesignTokens
import kotlinx.coroutines.launch
import uniffi.matrix_mobile.RoomItem
import uniffi.matrix_mobile.SyncState

class MainActivity : ComponentActivity() {
    private lateinit var session: Session

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        session = Session(applicationContext)
        // Extra CA for the disposable test servers, and (debug builds) credentials, arrive as intent extras.
        val ca = intent.getStringExtra("ca_b64")?.let { String(Base64.decode(it, Base64.DEFAULT)) }
        val debug = applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0
        val login = if (debug) Triple(intent.getStringExtra("homeserver"), intent.getStringExtra("user"), intent.getStringExtra("password")) else null
        val autoSend = if (debug) intent.getStringExtra("send") else null
        setContent {
            val p = Palette.current()
            LaunchedEffect(Unit) {
                session.restore(ca)
                if (!session.signedIn && login?.first != null && login.second != null && login.third != null)
                    session.signIn(login.first!!, login.second!!, login.third!!, ca)
            }
            var autoSent by remember { mutableStateOf(false) }
            LaunchedEffect(session.rooms) {
                val room = session.rooms.firstOrNull { !it.isSpace && !it.invited }
                if (!autoSent && autoSend != null && room != null) { autoSent = true; session.send(room, autoSend) }
            }
            Surface(Modifier.fillMaxSize().systemBarsPadding().imePadding(), color = p.surfaceColor) {
                if (session.signedIn) RoomList(session, p) else SignIn(session, p, ca)
            }
        }
    }
}

@Composable
fun SignIn(session: Session, p: Palette, ca: String?) {
    var homeserver by remember { mutableStateOf("https://") }
    var user by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val go: () -> Unit = {
        busy = true
        scope.launch { session.signIn(homeserver, user, password, ca); busy = false }
    }
    Column(Modifier.padding((DesignTokens.SPACING_ROW_PAD_Y * 2).dp), verticalArrangement = Arrangement.spacedBy(DesignTokens.SPACING_ROW_PAD_Y.dp)) {
        Text("Accedi", color = p.textColor, fontSize = 22.sp, fontWeight = FontWeight.Bold)
        OutlinedTextField(homeserver, { homeserver = it }, Modifier.fillMaxWidth(), label = { Text("Server") }, singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next))
        OutlinedTextField(user, { user = it }, Modifier.fillMaxWidth(), label = { Text("Utente (@nome:server)") }, singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next))
        OutlinedTextField(password, { password = it }, Modifier.fillMaxWidth(), label = { Text("Password") }, singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Go),
            keyboardActions = KeyboardActions(onGo = { go() }))
        if (session.error.isNotEmpty()) Text(session.error, color = p.errorColor)
        Button(go, Modifier.fillMaxWidth(), enabled = !busy && user.isNotEmpty() && password.isNotEmpty(),
            colors = ButtonDefaults.buttonColors(containerColor = p.accentColor, contentColor = p.onAccentColor),
            shape = RoundedCornerShape(DesignTokens.SPACING_TEXT_GAP.dp)) { Text(if (busy) "Accesso…" else "Accedi") }
    }
}

@Composable
fun RoomList(session: Session, p: Palette) {
    var target by remember { mutableStateOf<RoomItem?>(null) }
    Column(Modifier.fillMaxSize()) {
        Column(Modifier.padding(DesignTokens.SPACING_ROW_PAD_Y.dp)) {
            Text("Chat", color = p.textColor, fontSize = 22.sp, fontWeight = FontWeight.Bold)
            Text(when (session.state) {
                SyncState.CONNECTING -> "Connessione…"
                SyncState.READY -> "Connesso"
                SyncState.AUTHENTICATION_FAILED -> "Accesso scaduto"
                SyncState.SYNC_FAILED -> "Sincronizzazione non riuscita"
            }, color = p.secondaryColor, fontSize = 12.sp)
        }
        LazyColumn(Modifier.weight(1f)) {
            items(session.rooms.filter { !it.isSpace }, key = { it.roomId }) { room ->
                Row(Modifier.fillMaxWidth().clickable { target = room }
                    .padding(horizontal = DesignTokens.SPACING_ROW_PAD_Y.dp, vertical = DesignTokens.SPACING_ROW_PAD_Y.dp)) {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(DesignTokens.SPACING_TEXT_GAP.dp)) {
                        Text(room.name, color = p.textColor, fontWeight = FontWeight.SemiBold, fontSize = 17.sp)
                        room.preview?.let { Text(it, color = p.secondaryColor, maxLines = 1, fontSize = 14.sp) }
                    }
                    if (room.unread > 0uL) Text("${room.unread}", color = p.onAccentColor, fontSize = 12.sp,
                        modifier = Modifier.background(p.accentColor, RoundedCornerShape(50)).padding(DesignTokens.SPACING_TEXT_GAP.dp * 1.5f))
                }
                HorizontalDivider(color = p.borderColor)
            }
        }
        target?.let { room -> SendBar(session, room, p) { target = null } }
    }
}

@Composable
fun SendBar(session: Session, room: RoomItem, p: Palette, close: () -> Unit) {
    var text by remember(room.roomId) { mutableStateOf("") }
    val send = { if (text.isNotBlank()) { session.send(room, text); text = "" } }
    Column(Modifier.fillMaxWidth().padding(DesignTokens.SPACING_ROW_PAD_Y.dp), verticalArrangement = Arrangement.spacedBy(DesignTokens.SPACING_TEXT_GAP.dp)) {
        Text(room.name, color = p.textColor, fontWeight = FontWeight.Bold)
        OutlinedTextField(text, { text = it }, Modifier.fillMaxWidth(), label = { Text("Messaggio") },
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send), keyboardActions = KeyboardActions(onSend = { send() }))
        if (session.lastSend.isNotEmpty()) Text(session.lastSend, color = p.secondaryColor)
        Row { TextButton(close) { Text("Chiudi") }; Spacer(Modifier.weight(1f)); Button(send, enabled = text.isNotBlank()) { Text("Invia") } }
    }
}
