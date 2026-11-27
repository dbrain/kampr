package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KField
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.PrimaryAction
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.wire.PendingField
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

// A form's fields, each typed as the field says. Send waits until every required one is filled and
// every filled one fits; a field left empty is left out of the values.
@Composable
internal fun PendingForm(
    fields: List<PendingField>,
    enabled: Boolean,
    onSend: (JsonObject) -> Unit,
    onCancel: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val texts = remember(fields) { mutableStateMapOf<String, String>() }
    val check = formCheck(fields, texts)
    val send = { check.values?.takeIf { enabled }?.let(onSend) ?: Unit }
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(7.dp)) {
        // The fields scroll and the Send under them does not: a form taller than the card it is in
        // would otherwise scroll its own Send out of sight.
        val scroll = rememberScrollState()
        Box(Modifier.fillMaxWidth().weight(1f, fill = false)) {
            Column(
                Modifier.fillMaxWidth().verticalScroll(scroll),
                verticalArrangement = Arrangement.spacedBy(7.dp),
            ) {
                fields.forEach { field ->
                    val choices = choicesOf(field)
                    if (choices.isEmpty()) {
                        // A label above the box rather than a placeholder in it: a placeholder is cut to
                        // the box's width and gone at the first key, and "optional" was the word it lost.
                        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            KText(hint(field), tokens.type.micro, tokens.color.dim, maxLines = 2)
                            KField(
                                hint = typeHint(field),
                                text = texts[field.name].orEmpty(),
                                label = "Form field: ${field.label}",
                                onSubmit = send,
                            ) { texts[field.name] = it }
                        }
                    } else {
                        Choices(field, choices, texts[field.name], enabled) { texts[field.name] = if (texts[field.name] == it) "" else it }
                    }
                }
                check.problem?.let { KText(it, tokens.type.micro, tokens.color.blocked, maxLines = 2) }
            }
            MoreBelow(scroll, tokens.color.blockedBg, Modifier.align(Alignment.BottomStart))
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(7.dp)) {
            QuietAction("Cancel", onCancel, Modifier.weight(1f), label = "Put the form away")
            PrimaryAction(
                "Send",
                send,
                Modifier.weight(1f),
                tokens.type.buttonSmall,
                9.dp,
                enabled = enabled && check.values != null,
                label = "Send the form",
            )
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Choices(field: PendingField, choices: List<String>, chosen: String?, enabled: Boolean, onChoose: (String) -> Unit) {
    val tokens = Kampr.tokens
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        KText(hint(field), tokens.type.micro, tokens.color.dim, maxLines = 2)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(7.dp), verticalArrangement = Arrangement.spacedBy(7.dp)) {
            choices.forEach { choice ->
                val lit = choice == chosen
                AnswerChip("Choose $choice for ${field.label}", lit, enabled, { onChoose(choice) }, selected = lit) { ink ->
                    KText(choice, tokens.type.buttonSmall, ink)
                }
            }
        }
    }
}

private fun choicesOf(field: PendingField): List<String> = when {
    field.choices.isNotEmpty() -> field.choices
    field.type == "boolean" -> listOf("yes", "no")
    else -> emptyList()
}

private fun hint(field: PendingField): String = listOfNotNull(
    field.label,
    field.detail,
    "optional".takeIf { !field.required },
).joinToString(" · ")

private fun typeHint(field: PendingField): String = when (field.type) {
    "integer" -> "a whole number"
    "number" -> "a number"
    else -> "type here"
}

internal class FormCheck(val values: JsonObject?, val problem: String?)

internal fun formCheck(fields: List<PendingField>, texts: Map<String, String>): FormCheck {
    val values = mutableMapOf<String, JsonElement>()
    var missing = false
    for (field in fields) {
        val text = texts[field.name].orEmpty().trim()
        if (text.isEmpty()) {
            missing = missing || field.required
            continue
        }
        values[field.name] = valueOf(field, text) ?: return FormCheck(null, problemOf(field))
    }
    return FormCheck(if (missing) null else JsonObject(values), null)
}

private fun valueOf(field: PendingField, text: String): JsonElement? = when (field.type) {
    "integer" -> text.toLongOrNull()?.let { JsonPrimitive(it) }
    "number" -> text.toDoubleOrNull()?.let { JsonPrimitive(it) }
    "boolean" -> JsonPrimitive(text == "yes")
    else -> JsonPrimitive(text)
}

private fun problemOf(field: PendingField): String = when (field.type) {
    "integer" -> "${field.label} is a whole number"
    else -> "${field.label} is a number"
}
