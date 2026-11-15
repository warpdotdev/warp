import AppKit
import SwiftTerm

@MainActor
enum TerminalAppearance {
    static let background = NSColor(srgbRed: 7 / 255, green: 7 / 255, blue: 7 / 255, alpha: 1)
    static let foreground = NSColor(srgbRed: 0.95, green: 0.95, blue: 0.97, alpha: 1)
    static let muted = NSColor(white: 0.60, alpha: 1)
    static let separator = NSColor(white: 0.15, alpha: 1)
    static let command = NSColor(srgbRed: 0.71, green: 0.98, blue: 0.43, alpha: 1)
    static let failure = NSColor(srgbRed: 1, green: 0.49, blue: 0.44, alpha: 1)
    static let failureBackground = NSColor(srgbRed: 0.13, green: 0.075, blue: 0.065, alpha: 1)
    static let font = NSFont.monospacedSystemFont(ofSize: 14, weight: .regular)
    static let heading = NSFont.monospacedSystemFont(ofSize: 14, weight: .semibold)
    static let context = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
}

final class CommandHistory: NSScrollView {
    private final class Document: NSView {
        override var isFlipped: Bool { true }
    }

    private let timeline = NSStackView()
    private var blocks: [NSView] = []
    private var selected: Int?

    override init(frame: NSRect) {
        super.init(frame: frame)
        drawsBackground = true
        backgroundColor = TerminalAppearance.background
        hasVerticalScroller = true
        autohidesScrollers = true
        scrollerStyle = .overlay
        let document = Document()
        documentView = document
        timeline.orientation = .vertical
        timeline.alignment = .leading
        timeline.spacing = 0
        for view in [document, timeline] {
            view.translatesAutoresizingMaskIntoConstraints = false
        }
        document.addSubview(timeline)
        let fill = timeline.topAnchor.constraint(equalTo: document.topAnchor)
        fill.priority = NSLayoutConstraint.Priority(1)
        NSLayoutConstraint.activate([
            document.topAnchor.constraint(equalTo: contentView.topAnchor),
            document.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            document.heightAnchor.constraint(greaterThanOrEqualTo: contentView.heightAnchor),
            timeline.bottomAnchor.constraint(equalTo: document.bottomAnchor),
            timeline.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            timeline.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            timeline.topAnchor.constraint(greaterThanOrEqualTo: document.topAnchor),
            fill,
        ])
    }

    required init?(coder: NSCoder) { nil }

    func append(command: String, directory: String, duration: TimeInterval, exitCode: Int, output: NSAttributedString) {
        let block = NSView()
        block.wantsLayer = true
        block.layer?.backgroundColor = (exitCode == 0
            ? TerminalAppearance.background : TerminalAppearance.failureBackground).cgColor

        let separator = NSView()
        separator.wantsLayer = true
        separator.layer?.backgroundColor = TerminalAppearance.separator.cgColor
        let gutter = NSView()
        gutter.wantsLayer = true
        gutter.layer?.backgroundColor = exitCode == 0 ? nil : TerminalAppearance.failure.cgColor

        let context = NSTextField(labelWithString: "\(directory)  (\(String(format: "%.3f", duration))s)")
        context.font = TerminalAppearance.context
        context.textColor = TerminalAppearance.muted
        context.lineBreakMode = .byTruncatingMiddle
        let heading = Self.label()
        let title = NSMutableAttributedString(
            string: command, attributes: [.font: TerminalAppearance.heading,
                                          .foregroundColor: TerminalAppearance.foreground]
        )
        if let word = command.split(whereSeparator: \.isWhitespace).first,
           let range = command.range(of: word) {
            title.addAttribute(.foregroundColor, value: TerminalAppearance.command, range: NSRange(range, in: command))
        }
        heading.attributedStringValue = title

        let content = NSStackView(views: [context, heading])
        content.orientation = .vertical
        content.alignment = .leading
        content.spacing = 4
        if output.length > 0 {
            let result = Self.label()
            result.attributedStringValue = output
            content.addArrangedSubview(result)
            result.widthAnchor.constraint(equalTo: content.widthAnchor).isActive = true
            content.setCustomSpacing(8, after: heading)
        }
        block.addSubview(content)
        block.addSubview(separator)
        block.addSubview(gutter)
        for view in [content, separator, gutter] {
            view.translatesAutoresizingMaskIntoConstraints = false
        }
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: block.leadingAnchor, constant: 16),
            content.trailingAnchor.constraint(equalTo: block.trailingAnchor, constant: -16),
            content.topAnchor.constraint(equalTo: block.topAnchor, constant: 15),
            content.bottomAnchor.constraint(equalTo: block.bottomAnchor, constant: -15),
            context.widthAnchor.constraint(equalTo: content.widthAnchor),
            heading.widthAnchor.constraint(equalTo: content.widthAnchor),
            separator.topAnchor.constraint(equalTo: block.topAnchor),
            separator.leadingAnchor.constraint(equalTo: block.leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: block.trailingAnchor),
            separator.heightAnchor.constraint(equalToConstant: 1),
            gutter.leadingAnchor.constraint(equalTo: block.leadingAnchor),
            gutter.topAnchor.constraint(equalTo: block.topAnchor),
            gutter.bottomAnchor.constraint(equalTo: block.bottomAnchor),
            gutter.widthAnchor.constraint(equalToConstant: 4),
        ])
        timeline.addArrangedSubview(block)
        block.widthAnchor.constraint(equalTo: timeline.widthAnchor).isActive = true
        blocks.append(block)
        if blocks.count > 250 {
            blocks.removeFirst().removeFromSuperview()
        }
        selected = nil
    }

    func scrollToLatest() {
        layoutSubtreeIfNeeded()
        guard let documentView else { return }
        contentView.scroll(to: NSPoint(x: 0, y: max(0, documentView.frame.height - contentView.bounds.height)))
        reflectScrolledClipView(contentView)
    }

    func jump(previous: Bool) {
        guard !blocks.isEmpty else { return }
        let index = selected ?? blocks.count
        let target = min(blocks.count - 1, max(0, index + (previous ? -1 : 1)))
        selected = target
        blocks[target].scrollToVisible(blocks[target].bounds)
    }

    private static func label() -> NSTextField {
        let label = NSTextField(wrappingLabelWithString: "")
        label.isSelectable = true
        label.lineBreakMode = .byCharWrapping
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return label
    }

    static func output(from snapshot: TerminalContentSnapshot, marker: String) -> NSAttributedString? {
        let rows = snapshot.rows
        guard let end = rows.lastIndex(where: { $0.text == marker + "E" }) else { return nil }
        let start = rows[..<end].lastIndex(where: { $0.text == marker + "S" })
        let content = rows[(start.map { $0 + 1 } ?? rows.startIndex)..<end]
        let result = NSMutableAttributedString()
        if start == nil {
            result.append(NSAttributedString(string: "… earlier output is in the live terminal …\n", attributes: [
                .font: TerminalAppearance.context, .foregroundColor: TerminalAppearance.muted,
            ]))
        }
        var lines = content
        // The end marker starts on a new row even when output already ended with a newline.
        while let last = lines.last, last.text.allSatisfy(\.isWhitespace) { lines = lines.dropLast() }
        for (index, row) in lines.enumerated() {
            if index > 0, !row.isWrapped {
                result.append(NSAttributedString(string: "\n", attributes: [.font: TerminalAppearance.font]))
            }
            guard let last = row.cells.lastIndex(where: { !$0.isBlank }) else { continue }
            var run = ""
            var current: Attribute?
            for cell in row.cells[...last] where cell.width != 0 {
                if current != cell.attribute {
                    if let current {
                        result.append(Self.styled(run, attribute: current, palette: snapshot.palette))
                    }
                    current = cell.attribute
                    run = ""
                }
                run += cell.isBlank || cell.attribute.style.contains(.invisible) ? " " : cell.text
            }
            if let current {
                result.append(Self.styled(run, attribute: current, palette: snapshot.palette))
            }
        }
        return result
    }

    private static func styled(_ text: String, attribute: Attribute, palette: [SwiftTerm.Color]) -> NSAttributedString {
        let foreground: NSColor
        switch attribute.fg {
        case let .trueColor(red, green, blue):
            foreground = NSColor(srgbRed: CGFloat(red) / 255, green: CGFloat(green) / 255, blue: CGFloat(blue) / 255, alpha: 1)
        case let .ansi256(code) where palette.indices.contains(Int(code)):
            let color = palette[Int(code)]
            foreground = NSColor(srgbRed: CGFloat(color.red) / 65535, green: CGFloat(color.green) / 65535,
                                 blue: CGFloat(color.blue) / 65535, alpha: 1)
        default:
            foreground = TerminalAppearance.foreground
        }
        return NSAttributedString(string: text, attributes: [
            .font: attribute.style.contains(.bold) ? TerminalAppearance.heading : TerminalAppearance.font,
            .foregroundColor: foreground,
        ])
    }
}
