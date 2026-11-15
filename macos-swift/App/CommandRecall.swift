import AppKit

@MainActor
final class CommandRecall {
    struct Entry: Codable, Equatable {
        let command: String
        let session: String
        let date: Date?
    }

    static let shared = CommandRecall()
    private static let storageKey = "commandRecall.v1"
    private static let preferSessionKey = "commandRecall.preferCurrentSession"
    private let defaults: UserDefaults
    private(set) var entries: [Entry]

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        entries = defaults.data(forKey: Self.storageKey)
            .flatMap { try? JSONDecoder().decode([Entry].self, from: $0) } ?? []
    }

    var preferCurrentSession: Bool {
        get { defaults.object(forKey: Self.preferSessionKey) as? Bool ?? true }
        set { defaults.set(newValue, forKey: Self.preferSessionKey) }
    }

    func record(_ command: String, session: String) {
        guard let first = command.first, !first.isWhitespace, command.utf8.count <= 8192 else { return }
        entries.removeAll { $0.command == command }
        entries.insert(Entry(command: command, session: session, date: Date()), at: 0)
        if entries.count > 1000 {
            entries.removeLast(entries.count - 1000)
        }
        if let encoded = try? JSONEncoder().encode(entries) {
            defaults.set(encoded, forKey: Self.storageKey)
        }
    }

    func matches(_ query: String, session: String) -> [Entry] {
        let candidates = entries.filter { query.isEmpty || $0.command.localizedCaseInsensitiveContains(query) }
        guard preferCurrentSession else { return candidates }
        return candidates.filter { $0.session == session } + candidates.filter { $0.session != session }
    }

    func continuation(after prefix: String, session: String) -> String? {
        guard !prefix.isEmpty else { return nil }
        func extends(_ entry: Entry) -> Bool {
            entry.command.hasPrefix(prefix) && entry.command.count > prefix.count
        }
        let preferred = preferCurrentSession ? entries.first { $0.session == session && extends($0) } : nil
        return (preferred ?? entries.first(where: extends)).map { String($0.command.dropFirst(prefix.count)) }
    }
}

final class CommandChoiceList: NSView {
    private let rows = NSStackView()
    var accept: ((Int) -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = NSColor(white: 0.08, alpha: 1).cgColor
        layer?.cornerRadius = 7
        layer?.borderColor = TerminalAppearance.separator.cgColor
        layer?.borderWidth = 1
        setAccessibilityRole(.list)
        rows.orientation = .vertical
        rows.alignment = .leading
        rows.spacing = 1
        rows.edgeInsets = NSEdgeInsets(top: 5, left: 5, bottom: 5, right: 5)
        rows.translatesAutoresizingMaskIntoConstraints = false
        addSubview(rows)
        NSLayoutConstraint.activate([
            rows.topAnchor.constraint(equalTo: topAnchor),
            rows.bottomAnchor.constraint(equalTo: bottomAnchor),
            rows.leadingAnchor.constraint(equalTo: leadingAnchor),
            rows.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])
    }

    required init?(coder: NSCoder) { nil }

    func update(title: String, choices: [String], selected: Int) {
        setAccessibilityLabel(title)
        for row in rows.arrangedSubviews {
            row.removeFromSuperview()
        }
        if choices.isEmpty {
            let empty = NSTextField(labelWithString: "No matching commands")
            empty.textColor = TerminalAppearance.muted
            empty.font = TerminalAppearance.context
            rows.addArrangedSubview(empty)
            return
        }

        for (index, choice) in choices.enumerated() {
            let title = choice.replacingOccurrences(of: "\n", with: " ↵ ")
            let row = NSButton(title: title, target: self, action: #selector(choose(_:)))
            row.tag = index
            row.isBordered = false
            row.alignment = .left
            row.font = TerminalAppearance.font
            row.lineBreakMode = .byTruncatingTail
            row.setAccessibilityValue(index == selected ? "Selected" : "")
            if index == selected {
                row.wantsLayer = true
                row.layer?.backgroundColor = NSColor(white: 0.21, alpha: 1).cgColor
                row.layer?.cornerRadius = 4
            }
            rows.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: rows.widthAnchor, constant: -10).isActive = true
            row.heightAnchor.constraint(equalToConstant: 23).isActive = true
        }
    }

    @objc private func choose(_ sender: NSButton) {
        accept?(sender.tag)
    }
}

final class CommandHistoryPanel: NSView, NSTableViewDataSource, NSTableViewDelegate {
    private let table = NSTableView()
    private let noResults = NSTextField(labelWithString: "No matching commands")
    private let dateFormatter = RelativeDateTimeFormatter()
    private var entries: [CommandRecall.Entry] = []
    private var selectedRow: Int?
    var preview: ((Int) -> Void)?
    var accept: ((Int) -> Void)?
    var configurationChanged: (() -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = TerminalAppearance.background.cgColor
        setAccessibilityLabel("Command History")

        let title = NSTextField(labelWithString: "HISTORY")
        title.textColor = TerminalAppearance.foreground
        title.font = TerminalAppearance.context
        let configure = NSButton(title: "Configure", target: self, action: #selector(configureHistory(_:)))
        configure.image = NSImage(systemSymbolName: "slider.horizontal.3", accessibilityDescription: nil)
        configure.imagePosition = .imageLeading
        configure.isBordered = false
        configure.font = .systemFont(ofSize: 12, weight: .medium)
        configure.setAccessibilityLabel("Configure History")

        let header = NSView()
        header.wantsLayer = true
        header.layer?.backgroundColor = NSColor(white: 0.07, alpha: 1).cgColor
        header.addSubview(title)
        header.addSubview(configure)

        let scroller = NSScrollView()
        scroller.drawsBackground = false
        scroller.hasVerticalScroller = true
        scroller.autohidesScrollers = true
        scroller.scrollerStyle = .overlay
        scroller.documentView = table

        table.headerView = nil
        table.backgroundColor = TerminalAppearance.background
        table.selectionHighlightStyle = .none
        table.rowHeight = 21
        table.intercellSpacing = .zero
        table.style = .plain
        table.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("command"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.action = #selector(rowClicked)
        table.doubleAction = #selector(rowAccepted)
        table.setAccessibilityLabel("History results")

        let divider = NSView()
        divider.wantsLayer = true
        divider.layer?.backgroundColor = TerminalAppearance.separator.cgColor
        let hints = NSTextField(labelWithString: "")
        hints.attributedStringValue = Self.keyboardHints()
        noResults.font = TerminalAppearance.context
        noResults.textColor = TerminalAppearance.muted
        noResults.isHidden = true

        for view in [header, scroller, noResults, divider, hints] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        for view in [title, configure] { view.translatesAutoresizingMaskIntoConstraints = false }
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: topAnchor),
            header.leadingAnchor.constraint(equalTo: leadingAnchor),
            header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.heightAnchor.constraint(equalToConstant: 25),
            title.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 16),
            title.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            configure.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -16),
            configure.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            scroller.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 8),
            scroller.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            scroller.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            scroller.bottomAnchor.constraint(equalTo: divider.topAnchor, constant: -8),
            noResults.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16),
            noResults.centerYAnchor.constraint(equalTo: scroller.centerYAnchor),
            divider.leadingAnchor.constraint(equalTo: leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: trailingAnchor),
            divider.heightAnchor.constraint(equalToConstant: 1),
            divider.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -29),
            hints.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16),
            hints.centerYAnchor.constraint(equalTo: divider.bottomAnchor, constant: 14),
        ])
    }

    required init?(coder: NSCoder) { nil }

    func show(entries: [CommandRecall.Entry]) {
        self.entries = entries
        selectedRow = nil
        noResults.isHidden = !entries.isEmpty
        table.reloadData()
    }

    func highlight(_ row: Int?, scrollToEnd: Bool = false) {
        let before = selectedRow
        selectedRow = row
        for index in Set([before, row].compactMap { $0 }) where entries.indices.contains(index) {
            (table.view(atColumn: 0, row: index, makeIfNecessary: false) as? HistoryRow)?.highlighted = index == row
        }
        if let target = row ?? (scrollToEnd && !entries.isEmpty ? entries.count - 1 : nil) {
            table.scrollRowToVisible(target)
        }
    }

    func numberOfRows(in tableView: NSTableView) -> Int { entries.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let identifier = NSUserInterfaceItemIdentifier("historyRow")
        let view = tableView.makeView(withIdentifier: identifier, owner: self) as? HistoryRow ?? HistoryRow()
        view.identifier = identifier
        let entry = entries[row]
        view.command.stringValue = entry.command.replacingOccurrences(of: "\n", with: " ↵ ")
        view.time.stringValue = entry.date.map { date in
            let now = Date()
            if abs(date.timeIntervalSince(now)) < 1 { return "just now" }
            return dateFormatter.localizedString(for: date, relativeTo: now)
        } ?? ""
        view.highlighted = row == selectedRow
        view.setAccessibilityLabel(view.command.stringValue)
        return view
    }

    @objc private func rowClicked() {
        if entries.indices.contains(table.clickedRow) { preview?(table.clickedRow) }
    }

    @objc private func rowAccepted() {
        if entries.indices.contains(table.clickedRow) { accept?(table.clickedRow) }
    }

    @objc private func configureHistory(_ sender: NSButton) {
        let menu = NSMenu()
        let item = NSMenuItem(title: "Prefer Current Session", action: #selector(toggleSessionPriority), keyEquivalent: "")
        item.target = self
        item.state = CommandRecall.shared.preferCurrentSession ? .on : .off
        menu.addItem(item)
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.minY), in: sender)
    }

    @objc private func toggleSessionPriority() {
        CommandRecall.shared.preferCurrentSession.toggle()
        configurationChanged?()
    }

    private static func keyboardHints() -> NSAttributedString {
        let result = NSMutableAttributedString()
        let key: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .medium),
            .foregroundColor: TerminalAppearance.foreground,
            .backgroundColor: NSColor(white: 0.17, alpha: 1),
        ]
        let hint: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 11),
            .foregroundColor: TerminalAppearance.muted,
        ]
        result.append(NSAttributedString(string: " ↑ ", attributes: key))
        result.append(NSAttributedString(string: " ", attributes: hint))
        result.append(NSAttributedString(string: " ↓ ", attributes: key))
        result.append(NSAttributedString(string: " to navigate   ", attributes: hint))
        result.append(NSAttributedString(string: " esc ", attributes: key))
        result.append(NSAttributedString(string: " to dismiss", attributes: hint))
        return result
    }

    private final class HistoryRow: NSView {
        let command = NSTextField(labelWithString: "")
        let time = NSTextField(labelWithString: "")
        private let content = NSView()

        var highlighted = false {
            didSet {
                content.layer?.backgroundColor = highlighted ? NSColor(white: 0.12, alpha: 1).cgColor : nil
                setAccessibilityValue(highlighted ? "Selected" : "")
            }
        }

        init() {
            super.init(frame: .zero)
            setAccessibilityElement(true)
            setAccessibilityRole(.row)
            content.wantsLayer = true
            content.layer?.cornerRadius = 4

            let icon = NSTextField(labelWithString: "❯_")
            icon.font = NSFont.monospacedSystemFont(ofSize: 8, weight: .semibold)
            icon.textColor = TerminalAppearance.muted
            icon.alignment = .center
            icon.wantsLayer = true
            icon.layer?.backgroundColor = NSColor(white: 0.09, alpha: 1).cgColor
            icon.layer?.cornerRadius = 3
            command.font = TerminalAppearance.context
            command.textColor = TerminalAppearance.foreground
            command.lineBreakMode = .byTruncatingTail
            command.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            time.font = .systemFont(ofSize: 11)
            time.textColor = TerminalAppearance.muted
            time.alignment = .right
            time.setContentHuggingPriority(.required, for: .horizontal)

            content.translatesAutoresizingMaskIntoConstraints = false
            addSubview(content)
            for view in [icon, command, time] {
                view.translatesAutoresizingMaskIntoConstraints = false
                content.addSubview(view)
            }
            NSLayoutConstraint.activate([
                content.leadingAnchor.constraint(equalTo: leadingAnchor),
                content.trailingAnchor.constraint(equalTo: trailingAnchor),
                content.topAnchor.constraint(equalTo: topAnchor),
                content.bottomAnchor.constraint(equalTo: bottomAnchor),
                icon.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 8),
                icon.widthAnchor.constraint(equalToConstant: 15),
                icon.heightAnchor.constraint(equalToConstant: 15),
                icon.centerYAnchor.constraint(equalTo: content.centerYAnchor),
                command.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 8),
                command.centerYAnchor.constraint(equalTo: content.centerYAnchor),
                command.trailingAnchor.constraint(equalTo: time.leadingAnchor, constant: -12),
                time.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -10),
                time.centerYAnchor.constraint(equalTo: content.centerYAnchor),
            ])
        }

        required init?(coder: NSCoder) { nil }
    }
}
