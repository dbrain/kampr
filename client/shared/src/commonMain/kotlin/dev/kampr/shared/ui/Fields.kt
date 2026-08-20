package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr

@Composable
fun KField(
    hint: String,
    value: TextFieldValue,
    onChange: (TextFieldValue) -> Unit,
    modifier: Modifier = Modifier,
    style: TextStyle = Kampr.tokens.type.meta,
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
        if (value.text.isEmpty()) KText(hint, style, tokens.color.mute)
        BasicTextField(
            value = value,
            onValueChange = onChange,
            singleLine = true,
            textStyle = style.copy(color = tokens.color.text),
            cursorBrush = SolidColor(tokens.color.accent),
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
fun LabelledField(
    label: String,
    hint: String,
    value: TextFieldValue,
    onChange: (TextFieldValue) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(5.dp)) {
        LabelText(label, tokens.type.micro, tokens.color.mute)
        KField(hint, value, onChange)
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
                KField("NAME", TextFieldValue(key), { onChange(index, it.text to value) }, Modifier.weight(1f))
                KField("value", TextFieldValue(value), { onChange(index, key to it.text) }, Modifier.weight(1.2f))
                IconGlyph(
                    KamprIcons.cross,
                    13.dp,
                    tokens.color.mute,
                    Modifier.clickable { onRemove(index) },
                )
            }
        }
        Row(
            Modifier.clickable(onClick = onAdd).padding(vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            IconGlyph(KamprIcons.plus, 12.dp, tokens.color.accent)
            KText("Add a variable", tokens.type.captionSmall, tokens.color.accent)
        }
    }
}
