package dev.mothr.mobile

import androidx.compose.runtime.Composable

/**
 * Returns an action that opens the platform's QR scanner, or null where there is none.
 * [onCode] gets the scanned text; [onError] gets a message when the scanner can't open.
 */
@Composable
expect fun rememberQrScanner(onCode: (String) -> Unit, onError: (String) -> Unit): (() -> Unit)?
