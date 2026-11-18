package dev.mothr.mobile

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.backhandler.BackHandler
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

@OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
@Suppress("DEPRECATION") // BackHandler → NavigationEventHandler once it is stable in Compose Multiplatform.
@Composable
fun App() {
    MothrTheme {
        val model = Mothr.model
        Box(Modifier.fillMaxSize().background(pal.bg)) {
            val screen = when {
                model.client == null -> "pair"
                model.stack.isNotEmpty() -> "chat"
                else -> "chats"
            }
            AnimatedContent(
                targetState = screen,
                transitionSpec = {
                    val forward = targetState == "chat"
                    (slideInHorizontally(tween(260)) { if (forward) it / 4 else -it / 4 } + fadeIn(tween(200))) togetherWith
                        (slideOutHorizontally(tween(260)) { if (forward) -it / 6 else it / 6 } + fadeOut(tween(160)))
                },
                label = "screen",
            ) { s ->
                when (s) {
                    "pair" -> PairScreen(model)
                    "chats" -> ChatsScreen(model)
                    else -> ChatScreen(model)
                }
            }

            BackHandler(enabled = model.stack.isNotEmpty() || model.showNewChat || model.showUsage) {
                when {
                    model.showNewChat -> model.showNewChat = false
                    model.showUsage -> model.showUsage = false
                    else -> model.back()
                }
            }

            if (model.showNewChat) NewChatSheet(model)
            if (model.showUsage) UsageSheet(model)

            // Toasts: brief, bottom, quiet.
            val toast = model.toast
            LaunchedEffect(toast) {
                if (toast != null) {
                    delay(3500)
                    model.toast = null
                }
            }
            AnimatedVisibility(
                visible = toast != null,
                enter = slideInVertically { it } + fadeIn(),
                exit = slideOutVertically { it } + fadeOut(),
                modifier = Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.safeDrawing).padding(16.dp, 0.dp, 16.dp, 96.dp),
            ) {
                Card(Modifier.fillMaxWidth(), bg = pal.raised, border = pal.borderStrong) {
                    Text(toast ?: "", style = type.body, modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp))
                }
            }
        }
    }
}
