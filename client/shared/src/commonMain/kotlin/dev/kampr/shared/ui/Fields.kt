package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr

// `onSubmit` is what puts a labelled action key on the phone's keyboard. Without one the IME
// shows a plain Return, a single-line field swallows it, and the field has no way at all to be
// finished from the keyboard that is covering the button.
@Composable
fun KField(
    hint: String,
    value: TextFieldValue,
    modifier: Modifier = Modifier,
    style: TextStyle = Kampr.tokens.type.meta,
    label: String? = null,
    keyboard: KeyboardOptions = KeyboardOptions.Default,
    onSubmit: (() -> Unit)? = null,
    onChange: (TextFieldValue) -> Unit,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Box(
        modifier
            .fillMaxWidth()
            .background(tokens.color.surface2, shape)
            .edge(tokens.card, shape)
            .padding(horizontal = 11.dp, vertical = 10.dp),
    ) {
        // A field already selects its own contents, with its own handles. A container above it
        // would draw a second selection over the same glyphs and drag the hint painted behind
        // them into the copy.
        DisableSelection {
            if (value.text.isEmpty()) KText(hint, style, tokens.color.mute)
        }
        BasicTextField(
            value = value,
            onValueChange = onChange,
            singleLine = true,
            textStyle = style.copy(color = tokens.color.text),
            cursorBrush = SolidColor(tokens.color.accent),
            keyboardOptions = if (onSubmit == null) keyboard else keyboard.copy(imeAction = ImeAction.Go),
            keyboardActions = KeyboardActions(onGo = onSubmit?.let { submit -> { submit() } }),
            modifier = Modifier.fillMaxWidth().named(label ?: hint),
        )
    }
}

@Composable
fun LabelledField(
    label: String,
    hint: String,
    value: TextFieldValue,
    modifier: Modifier = Modifier,
    onChange: (TextFieldValue) -> Unit,
) {
    val tokens = Kampr.tokens
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(5.dp)) {
        LabelText(label, tokens.type.micro, tokens.color.mute)
        KField(hint, value, label = label, onChange = onChange)
    }
}

// `workspace.create` and `tab.create` take an env map, which is what makes "a Claude session in
// this worktree with these variables" one call rather than a scripted sequence (findings §3.12).
@Composable
fun EnvEditor(
    rows: List<Pair<String, String>>,
    onChange: (Int, Pair<String, String>) -> Unit,
    onAdd: () -> Unit,
    onRemove: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(5.dp)) {
        LabelText("environment", tokens.type.micro, tokens.color.mute)
        rows.forEachIndexed { index, (key, value) ->
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                KField("NAME", TextFieldValue(key), Modifier.weight(1f), label = "Variable name") {
                    onChange(index, it.text to value)
                }
                KField("value", TextFieldValue(value), Modifier.weight(1.2f), label = "Value of $key") {
                    onChange(index, key to it.text)
                }
                GlyphTarget(
                    KamprIcons.cross,
                    "Remove ${key.ifBlank { "this variable" }}",
                    tokens.color.mute,
                    { onRemove(index) },
                    target = LANDSCAPE_TOUCH,
                    glyph = 13.dp,
                )
            }
        }
        Row(
            Modifier.touchable(LANDSCAPE_TOUCH).action("Add a variable", onAdd).padding(vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            IconGlyph(KamprIcons.plus, 12.dp, tokens.color.accent)
            KText("Add a variable", tokens.type.captionSmall, tokens.color.accent)
        }
    }
}
