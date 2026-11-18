package dev.mothr.mobile

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.ripple
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.mothr.client.AgentRef
import dev.mothr.client.ChatStatus
import org.jetbrains.compose.resources.DrawableResource
import org.jetbrains.compose.resources.painterResource

@Composable
fun Ico(res: DrawableResource, tint: Color = pal.textMuted, size: Dp = 18.dp, modifier: Modifier = Modifier) {
    Icon(painterResource(res), contentDescription = null, tint = tint, modifier = modifier.size(size))
}

/** Modifier for tappable surfaces with Material ripple. */
@Composable
fun Modifier.tap(enabled: Boolean = true, shape: Shape = Radius.md, onClick: () -> Unit): Modifier =
    this.clip(shape).clickable(
        enabled = enabled,
        interactionSource = remember { MutableInteractionSource() },
        indication = ripple(),
        onClick = onClick,
    )

/** Agent tile: monogram on its brand tint. */
@Composable
fun AgentAvatar(agent: AgentRef, size: Dp = 32.dp, status: ChatStatus? = null) {
    val tint = agentColor(agent.color)
    Box {
        Box(
            Modifier.size(size).clip(RoundedCornerShape(size * 0.3f)).background(tint.copy(alpha = if (pal.dark) 0.18f else 0.14f)),
            contentAlignment = Alignment.Center,
        ) {
            Text(agent.monogram, style = type.label.copy(color = tint, fontSize = (size.value * 0.42f).sp, fontWeight = FontWeight.SemiBold))
        }
        if (status != null) {
            Box(
                Modifier.align(Alignment.BottomEnd).padding(start = 0.dp).size(12.dp).clip(CircleShape).background(pal.bg),
                contentAlignment = Alignment.Center,
            ) { StatusDot(status, 8.dp) }
        }
    }
}

fun statusColor(status: ChatStatus, p: Palette): Color = when (status) {
    ChatStatus.WORKING, ChatStatus.STARTING -> p.brand
    ChatStatus.NEEDS_YOU -> p.warning
    ChatStatus.ERROR -> p.danger
    ChatStatus.READY -> p.success
    ChatStatus.ENDED -> p.textFaint
}

/** Status dot; pulses while the chat is live and busy. */
@Composable
fun StatusDot(status: ChatStatus, size: Dp = 8.dp) {
    val color = statusColor(status, pal)
    val live = status == ChatStatus.WORKING || status == ChatStatus.NEEDS_YOU || status == ChatStatus.STARTING
    val pulse = if (live) {
        val t = rememberInfiniteTransition(label = "pulse")
        val a by t.animateFloat(0.35f, 1f, infiniteRepeatable(tween(900, easing = FastOutSlowInEasing), RepeatMode.Reverse), label = "a")
        a
    } else 1f
    Box(Modifier.size(size).alpha(pulse).clip(CircleShape).background(color))
}

fun levelColor(level: String, p: Palette): Color = when (level) {
    "warn" -> p.warning
    "critical", "exhausted" -> p.danger
    else -> p.success
}

/** A circular progress ring (usage, context). */
@Composable
fun Ring(fraction: Float, color: Color, size: Dp = 22.dp, stroke: Dp = 2.5.dp, track: Color = pal.border) {
    val f by animateFloatAsState(fraction.coerceIn(0f, 1f), tween(400), label = "ring")
    Canvas(Modifier.size(size)) {
        val w = stroke.toPx()
        val inset = w / 2
        val arcSize = Size(this.size.width - w, this.size.height - w)
        drawArc(track, 0f, 360f, false, Offset(inset, inset), arcSize, style = Stroke(w))
        if (f > 0f) drawArc(color, -90f, 360f * f, false, Offset(inset, inset), arcSize, style = Stroke(w, cap = StrokeCap.Round))
    }
}

/** Thin horizontal meter. */
@Composable
fun Meter(fraction: Float, color: Color, modifier: Modifier = Modifier) {
    val f by animateFloatAsState(fraction.coerceIn(0f, 1f), tween(400), label = "meter")
    val track = pal.border
    Canvas(modifier.height(5.dp)) {
        val r = androidx.compose.ui.geometry.CornerRadius(this.size.height / 2)
        drawRoundRect(track, cornerRadius = r)
        if (f > 0f) drawRoundRect(color, size = Size(this.size.width * f, this.size.height), cornerRadius = r)
    }
}

/** Small rounded label. */
@Composable
fun Pill(text: String, color: Color = pal.textMuted, bg: Color = pal.surfaceHover, icon: DrawableResource? = null, onClick: (() -> Unit)? = null) {
    Row(
        Modifier.clip(Radius.full).background(bg).then(if (onClick != null) Modifier.tap(shape = Radius.full, onClick = onClick) else Modifier)
            .padding(horizontal = 10.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        if (icon != null) Ico(icon, tint = color, size = 14.dp)
        Text(text, style = type.caption.copy(color = color))
    }
}

/** Buttons: primary is monochrome (text on bg), per DESIGN.md. */
enum class Tone { Primary, Quiet, Ghost, Danger, Warning }

@Composable
fun Btn(
    text: String,
    tone: Tone = Tone.Quiet,
    icon: DrawableResource? = null,
    enabled: Boolean = true,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    val p = pal
    val (bg, fg, border) = when (tone) {
        Tone.Primary -> Triple(p.text, p.bg, Color.Transparent)
        Tone.Quiet -> Triple(p.surface, p.text, p.borderStrong)
        Tone.Ghost -> Triple(Color.Transparent, p.textMuted, Color.Transparent)
        Tone.Danger -> Triple(p.danger.copy(alpha = 0.12f), p.danger, p.danger.copy(alpha = 0.35f))
        Tone.Warning -> Triple(p.warning, Color(0xFF1A1300), Color.Transparent)
    }
    Row(
        modifier.heightIn(min = 42.dp).clip(Radius.md).background(bg).border(1.dp, border, Radius.md)
            .tap(enabled = enabled, onClick = onClick).alpha(if (enabled) 1f else 0.45f).padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(7.dp, Alignment.CenterHorizontally),
    ) {
        if (icon != null) Ico(icon, tint = fg, size = 17.dp)
        Text(text, style = type.label.copy(color = fg))
    }
}

@Composable
fun IconBtn(res: DrawableResource, tint: Color = pal.textMuted, size: Dp = 40.dp, bg: Color = Color.Transparent, onClick: () -> Unit) {
    Box(Modifier.size(size).clip(CircleShape).background(bg).tap(shape = CircleShape, onClick = onClick), contentAlignment = Alignment.Center) {
        Ico(res, tint = tint, size = 20.dp)
    }
}

/** A bordered card surface. */
@Composable
fun Card(modifier: Modifier = Modifier, border: Color = pal.border, bg: Color = pal.surface, content: @Composable () -> Unit) {
    Box(modifier.clip(Radius.lg).background(bg).border(1.dp, border, Radius.lg)) { content() }
}

@Composable
fun RowScope.Spacer1() = androidx.compose.foundation.layout.Spacer(Modifier.weight(1f))
