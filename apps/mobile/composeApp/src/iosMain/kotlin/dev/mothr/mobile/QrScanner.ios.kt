package dev.mothr.mobile

import androidx.compose.runtime.Composable

/** No in-app scanner on iOS yet: pairing there uses the pasted link. */
@Composable
actual fun rememberQrScanner(onCode: (String) -> Unit, onError: (String) -> Unit): (() -> Unit)? = null
