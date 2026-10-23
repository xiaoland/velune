package app.velune.android

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import org.commonmark.ext.autolink.AutolinkExtension
import org.commonmark.ext.gfm.strikethrough.Strikethrough
import org.commonmark.ext.gfm.strikethrough.StrikethroughExtension
import org.commonmark.ext.gfm.tables.TableBlock
import org.commonmark.ext.gfm.tables.TableCell
import org.commonmark.ext.gfm.tables.TablesExtension
import org.commonmark.node.BlockQuote
import org.commonmark.node.BulletList
import org.commonmark.node.Code
import org.commonmark.node.Document
import org.commonmark.node.Emphasis
import org.commonmark.node.FencedCodeBlock
import org.commonmark.node.HardLineBreak
import org.commonmark.node.Heading
import org.commonmark.node.HtmlBlock
import org.commonmark.node.HtmlInline
import org.commonmark.node.Image
import org.commonmark.node.IndentedCodeBlock
import org.commonmark.node.Link
import org.commonmark.node.ListItem
import org.commonmark.node.Node
import org.commonmark.node.OrderedList
import org.commonmark.node.Paragraph
import org.commonmark.node.SoftLineBreak
import org.commonmark.node.StrongEmphasis
import org.commonmark.node.ThematicBreak
import org.commonmark.parser.Parser

private val parser: Parser = Parser.builder()
    .extensions(listOf(TablesExtension.create(), StrikethroughExtension.create(), AutolinkExtension.create()))
    .build()

private fun Node.children(): List<Node> = generateSequence(firstChild) { it.next }.toList()

/**
 * The model's reply as CommonMark with GitHub tables, strikethrough and
 * autolinks. Code blocks scroll sideways and copy whole; everything else
 * wraps. Parsing is cheap enough to redo as a streamed reply grows.
 */
@Composable
fun Markdown(text: String, modifier: Modifier = Modifier, style: TextStyle = MaterialTheme.typography.bodyLarge) {
    val document = remember(text) { parser.parse(text) as Document }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        document.children().forEach { MarkdownBlock(it, style) }
    }
}

@Composable
private fun MarkdownBlock(node: Node, style: TextStyle) {
    val colors = MaterialTheme.colorScheme
    val typography = MaterialTheme.typography
    when (node) {
        is Paragraph -> Text(inline(node), style = style)
        is Heading -> Text(
            inline(node),
            style = when (node.level) {
                1 -> typography.titleLarge
                2 -> typography.titleMedium
                else -> typography.titleSmall
            }.copy(fontWeight = FontWeight.SemiBold),
            modifier = Modifier.padding(top = 4.dp),
        )
        is BulletList -> ListBlock(node.children(), style) { "•" }
        is OrderedList -> {
            val start = node.markerStartNumber ?: 1
            ListBlock(node.children(), style) { index -> "${start + index}." }
        }
        is BlockQuote -> Row(Modifier.height(IntrinsicSize.Min)) {
            Box(Modifier.width(3.dp).fillMaxHeight().background(colors.outlineVariant, RoundedCornerShape(2.dp)))
            Column(Modifier.padding(start = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                node.children().forEach { MarkdownBlock(it, style.copy(color = colors.onSurfaceVariant)) }
            }
        }
        is FencedCodeBlock -> CodeBlock(node.literal.trimEnd('\n'), node.info?.substringBefore(' ')?.takeIf { it.isNotBlank() })
        is IndentedCodeBlock -> CodeBlock(node.literal.trimEnd('\n'), null)
        is HtmlBlock -> CodeBlock(node.literal.trimEnd('\n'), null)
        is ThematicBreak -> HorizontalDivider(Modifier.padding(vertical = 4.dp))
        is TableBlock -> Table(node, style)
        else -> node.children().forEach { MarkdownBlock(it, style) }
    }
}

@Composable
private fun ListBlock(items: List<Node>, style: TextStyle, marker: (Int) -> String) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        items.filterIsInstance<ListItem>().forEachIndexed { index, item ->
            Row {
                Text(marker(index), style = style, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.widthIn(min = 20.dp).padding(end = 6.dp))
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    item.children().forEach { MarkdownBlock(it, style) }
                }
            }
        }
    }
}

@Composable
fun CodeBlock(code: String, language: String?) {
    val copy = rememberCopy()
    Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = RoundedCornerShape(10.dp), modifier = Modifier.fillMaxWidth()) {
        Column {
            Row(Modifier.fillMaxWidth().padding(start = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(language ?: "", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
                IconButton(onClick = { copy(code) }, modifier = Modifier.size(36.dp)) {
                    Icon(Icons.Outlined.ContentCopy, contentDescription = "复制代码", modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            Text(
                code, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace, softWrap = false,
                modifier = Modifier.horizontalScroll(rememberScrollState()).padding(start = 12.dp, end = 12.dp, bottom = 12.dp),
            )
        }
    }
}

/** A grid: every column as wide as its widest cell (up to 240 dp), the header row shaded. */
@Composable
private fun Table(table: TableBlock, style: TextStyle) {
    val rows = table.children().flatMap { section -> section.children() }.map { row -> row.children().filterIsInstance<TableCell>() }
    val columns = rows.maxOfOrNull { it.size } ?: return
    val border = MaterialTheme.colorScheme.outlineVariant
    val headerColor = MaterialTheme.colorScheme.surfaceContainerHigh
    Box(Modifier.horizontalScroll(rememberScrollState())) {
        Layout(
            content = {
                rows.forEach { row ->
                    repeat(columns) { index ->
                        val cell = row.getOrNull(index)
                        Text(
                            cell?.let { inline(it) } ?: AnnotatedString(""),
                            style = MaterialTheme.typography.bodyMedium.copy(fontWeight = if (cell?.isHeader == true) FontWeight.SemiBold else style.fontWeight),
                            modifier = Modifier.background(if (cell?.isHeader == true) headerColor else Color.Transparent).padding(horizontal = 10.dp, vertical = 6.dp),
                        )
                    }
                }
            },
            modifier = Modifier.border(1.dp, border, RoundedCornerShape(6.dp)).clip(RoundedCornerShape(6.dp)),
        ) { measurables, constraints ->
            val max = 240.dp.roundToPx()
            val min = 48.dp.roundToPx()
            val widths = IntArray(columns) { column ->
                (column until measurables.size step columns).maxOf { measurables[it].maxIntrinsicWidth(constraints.maxHeight) }.coerceIn(min, max)
            }
            val placeables = measurables.mapIndexed { index, measurable ->
                val width = widths[index % columns]
                measurable.measure(Constraints.fixedWidth(width))
            }
            val heights = placeables.chunked(columns).map { row -> row.maxOf { it.height } }
            layout(widths.sum(), heights.sum()) {
                var y = 0
                placeables.chunked(columns).forEachIndexed { row, cells ->
                    var x = 0
                    cells.forEachIndexed { column, cell -> cell.place(x, y); x += widths[column] }
                    y += heights[row]
                }
            }
        }
    }
}

@Composable
private fun inline(node: Node): AnnotatedString {
    val code = SpanStyle(fontFamily = FontFamily.Monospace, fontSize = 0.9.em, background = MaterialTheme.colorScheme.surfaceContainerHigh)
    val link = TextLinkStyles(SpanStyle(color = MaterialTheme.colorScheme.primary, textDecoration = TextDecoration.Underline))
    return remember(node, code, link) {
        buildAnnotatedString {
            fun append(node: Node) {
                when (node) {
                    is org.commonmark.node.Text -> append(node.literal)
                    is Code -> withStyle(code) { append(node.literal) }
                    is Emphasis -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { node.children().forEach(::append) }
                    is StrongEmphasis -> withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) { node.children().forEach(::append) }
                    is Strikethrough -> withStyle(SpanStyle(textDecoration = TextDecoration.LineThrough)) { node.children().forEach(::append) }
                    is Link -> withLink(LinkAnnotation.Url(node.destination, link)) { node.children().forEach(::append) }
                    is Image -> node.children().forEach(::append)
                    // Models break lines on purpose more often than they wrap prose by hand.
                    is SoftLineBreak, is HardLineBreak -> append('\n')
                    is HtmlInline -> withStyle(code.copy(background = Color.Unspecified)) { append(node.literal) }
                    else -> node.children().forEach(::append)
                }
            }
            node.children().forEach(::append)
        }
    }
}
