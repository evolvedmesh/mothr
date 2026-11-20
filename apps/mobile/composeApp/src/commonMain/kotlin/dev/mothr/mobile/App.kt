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
                model.client == null || model.addingComputer -> "pair"
                model.showNewChat -> "new"
                model.stack.isNotEmpty() -> "chat"
                model.openProject != null -> "project"
                model.showProjects -> "projects"
                else -> "chats"
            }
            AnimatedContent(
                targetState = screen,
                transitionSpec = {
                    val forward = depth(targetState) > depth(initialState)
                    if (targetState == "new" || initialState == "new") {
                        // New chat rises over the list, like a compose window.
                        val rising = targetState == "new"
                        (slideInVertically(tween(240)) { if (rising) it / 8 else 0 } + fadeIn(tween(200))) togetherWith
                            (slideOutVertically(tween(200)) { if (rising) 0 else it / 8 } + fadeOut(tween(160)))
                    } else (slideInHorizontally(tween(260)) { if (forward) it / 4 else -it / 4 } + fadeIn(tween(200))) togetherWith
                        (slideOutHorizontally(tween(260)) { if (forward) -it / 6 else it / 6 } + fadeOut(tween(160)))
                },
                label = "screen",
            ) { s ->
                when (s) {
                    "pair" -> PairScreen(model)
                    "new" -> NewChatScreen(model)
                    "chats" -> ChatsScreen(model)
                    "projects" -> ProjectsScreen(model)
                    "project" -> ProjectScreen(model)
                    else -> ChatScreen(model)
                }
            }

            val inProjects = model.showProjects || model.openProject != null
            BackHandler(enabled = model.stack.isNotEmpty() || model.showNewChat || model.showUsage || model.addingComputer || inProjects) {
                when {
                    model.addingComputer -> model.cancelAdding()
                    model.showNewChat -> model.showNewChat = false
                    model.showUsage -> model.showUsage = false
                    model.stack.isNotEmpty() -> model.back()
                    else -> model.projectBack()
                }
            }

            if (model.showUsage) UsageSheet(model)
            if (model.showComputers) ComputersSheet(model)

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

/** How deep a screen sits, so moving deeper slides in from the right. */
private fun depth(screen: String) = when (screen) {
    "chats" -> 0
    "projects" -> 1
    "project" -> 2
    "chat" -> 3
    else -> 0
}
