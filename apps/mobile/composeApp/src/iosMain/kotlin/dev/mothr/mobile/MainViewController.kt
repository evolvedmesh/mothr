package dev.mothr.mobile

import androidx.compose.ui.window.ComposeUIViewController

/** Entry point used by the Swift host in `iosApp/`. */
@Suppress("FunctionName", "unused")
fun MainViewController() = ComposeUIViewController { App() }
