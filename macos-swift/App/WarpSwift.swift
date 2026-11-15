import AppKit
import SwiftTerm

@main
final class WarpSwift: NSObject, NSApplicationDelegate {
    private var windows: [TerminalWindow] = []

    static func main() {
        let app = NSApplication.shared
        let delegate = WarpSwift()
        app.delegate = delegate
        app.setActivationPolicy(.regular)
        withExtendedLifetime(delegate) { app.run() }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let mainMenu = NSMenu()
        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "Quit Warp Swift", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu
        mainMenu.addItem(appItem)

        let shellItem = NSMenuItem()
        let shellMenu = NSMenu(title: "Shell")
        shellMenu.addItem(withTitle: "New Window", action: #selector(newWindow), keyEquivalent: "n").target = self
        shellMenu.addItem(withTitle: "New Tab", action: #selector(newTab), keyEquivalent: "t").target = self
        shellMenu.addItem(withTitle: "Split Pane", action: #selector(TerminalWindow.splitPane), keyEquivalent: "d")
        shellMenu.addItem(withTitle: "Close Tab", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        let previous = shellMenu.addItem(
            withTitle: "Previous Command", action: #selector(TerminalWindow.previousCommand), keyEquivalent: "["
        )
        previous.keyEquivalentModifierMask = [.command, .shift]
        let next = shellMenu.addItem(
            withTitle: "Next Command", action: #selector(TerminalWindow.nextCommand), keyEquivalent: "]"
        )
        next.keyEquivalentModifierMask = [.command, .shift]
        shellItem.submenu = shellMenu
        mainMenu.addItem(shellItem)

        let editItem = NSMenuItem()
        let editMenu = NSMenu(title: "Edit")
        editMenu.addItem(withTitle: "Undo", action: Selector("undo:"), keyEquivalent: "z")
        let redo = editMenu.addItem(withTitle: "Redo", action: Selector("redo:"), keyEquivalent: "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        editMenu.addItem(.separator())
        editMenu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        editMenu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        editMenu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        editMenu.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        editMenu.addItem(withTitle: "Find in Terminal", action: #selector(TerminalWindow.findInTerminal), keyEquivalent: "f")
        editItem.submenu = editMenu
        mainMenu.addItem(editItem)

        NSApp.mainMenu = mainMenu
        newWindow()
        NSApp.activate()
    }

    @objc private func newWindow() {
        openWindow(tabbedWith: nil)
    }

    @objc fileprivate func newTab() {
        openWindow(tabbedWith: NSApp.keyWindow)
    }

    private func openWindow(tabbedWith parent: NSWindow?) {
        let window = TerminalWindow()
        windows.append(window)
        window.onClose = { [weak self, weak window] in
            self?.windows.removeAll { $0 === window }
        }
        parent?.addTabbedWindow(window, ordered: .above)
        window.makeKeyAndOrderFront(nil)
        window.splitPane()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

final class TerminalWindow: NSWindow, NSWindowDelegate {
    private let split = NSSplitView()
    var onClose: (() -> Void)?

    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 1000, height: 620),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        title = "Warp Swift"
        appearance = NSAppearance(named: .darkAqua)
        backgroundColor = TerminalAppearance.background
        titlebarAppearsTransparent = true
        titlebarSeparatorStyle = .line
        let addTab = NSButton(
            image: NSImage(systemSymbolName: "plus", accessibilityDescription: "New Tab")!,
            target: NSApp.delegate, action: #selector(WarpSwift.newTab)
        )
        addTab.isBordered = false
        addTab.toolTip = "New Tab (⌘T)"
        addTab.frame = NSRect(x: 0, y: 0, width: 36, height: 28)
        let tabControls = NSTitlebarAccessoryViewController()
        tabControls.view = addTab
        tabControls.layoutAttribute = .right
        addTitlebarAccessoryViewController(tabControls)
        isReleasedWhenClosed = false
        minSize = NSSize(width: 440, height: 260)
        tabbingIdentifier = "WarpSwiftTerminal"
        delegate = self
        split.isVertical = true
        split.dividerStyle = .thin
        contentView = split
        center()
    }

    func windowDidBecomeKey(_ notification: Notification) {
        tintNativeTabsAfterLayout()
    }

    private func tintNativeTabsAfterLayout() {
        guard #available(macOS 27, *), !styleMask.contains(.fullScreen),
              (tabbedWindows?.count ?? 0) > 1 else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self, isKeyWindow, !styleMask.contains(.fullScreen),
                  let contentView, let frame = contentView.superview else { return }
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            defer { CATransaction.commit() }
            @MainActor func tintTabs(in view: NSView, insideTabBar: Bool = false) {
                guard view !== contentView else { return }
                let isTabBar = insideTabBar || view.accessibilityRole() == .tabGroup
                if isTabBar {
                    if let glass = view as? NSGlassEffectView {
                        if glass.tintColor != TerminalAppearance.background {
                            glass.tintColor = TerminalAppearance.background
                        }
                    } else if String(describing: type(of: view)) == "NSSubduedGlassEffectView",
                              view.subviews.count == 3, let content = view.subviews.dropFirst().first {
                        // macOS 27's tab track has no public tint API. Paint the system's
                        // content plane above its backdrop and below the buttons.
                        if let layer = content.layer, layer.backgroundColor != TerminalAppearance.background.cgColor {
                            layer.backgroundColor = TerminalAppearance.background.cgColor
                            layer.cornerRadius = content.bounds.height / 2
                        }
                    }
                }
                for child in view.subviews {
                    tintTabs(in: child, insideTabBar: isTabBar)
                }
            }
            tintTabs(in: frame)
        }
    }

    @objc func splitPane() {
        let session = TerminalSession(frame: split.bounds)
        split.addArrangedSubview(session)
        split.adjustSubviews()
        session.start()
    }

    private var activeSession: TerminalSession? {
        guard let responder = firstResponder as? NSView else { return nil }
        return split.arrangedSubviews.compactMap { $0 as? TerminalSession }.first {
            responder.isDescendant(of: $0)
        }
    }

    @objc func findInTerminal() {
        activeSession?.showFind()
    }

    @objc func previousCommand() {
        activeSession?.jumpToCommand(previous: true)
    }

    @objc func nextCommand() {
        activeSession?.jumpToCommand(previous: false)
    }

    func windowWillClose(_ notification: Notification) {
        for case let session as TerminalSession in split.arrangedSubviews {
            session.close()
        }
        onClose?()
    }
}

private final class CommandEditor: NSTextView {
    var submit: (() -> Void)?
    var passthrough: (() -> Void)?
    var handleRecallKey: ((NSEvent) -> Bool)?
    var suggestion: String? { didSet { needsDisplay = true } }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard let suggestion, !hasMarkedText(), let layoutManager, let textContainer,
              layoutManager.numberOfGlyphs > 0 else { return }
        layoutManager.ensureLayout(for: textContainer)
        let glyph = layoutManager.numberOfGlyphs - 1
        let caret = layoutManager.boundingRect(forGlyphRange: NSRange(location: glyph, length: 1), in: textContainer)
        let origin = textContainerOrigin
        suggestion.draw(at: NSPoint(x: origin.x + caret.maxX, y: origin.y + caret.minY), withAttributes: [
            .font: TerminalAppearance.font, .foregroundColor: TerminalAppearance.muted,
        ])
    }

    override func keyDown(with event: NSEvent) {
        if !hasMarkedText(), handleRecallKey?(event) == true { return }
        let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if [36, 76].contains(event.keyCode), !hasMarkedText() {
            if modifiers.contains(.shift) && modifiers.contains(.command) {
                passthrough?()
                return
            }
            if modifiers.intersection([.shift, .option, .command, .control]).isEmpty {
                submit?()
                return
            }
            if modifiers.intersection([.shift, .option, .command, .control]) == .shift {
                insertNewlineIgnoringFieldEditor(nil)
                return
            }
        }
        super.keyDown(with: event)
    }
}

private final class TerminalSession: NSView, LocalProcessTerminalViewDelegate, NSTextViewDelegate {
    private let terminal = LocalProcessTerminalView(frame: .zero)
    private let history = CommandHistory(frame: .zero)
    private let recall = CommandRecall.shared
    private let recallList = CommandChoiceList(frame: .zero)
    private var recallListHeight: NSLayoutConstraint?
    private let historyPanel = CommandHistoryPanel(frame: .zero)
    private var historyPanelHeight: NSLayoutConstraint?
    private var recallDraft: (text: String, selection: NSRange)?
    private var recallQuery: (text: String, selection: NSRange)?
    private var previewingRecall = false
    private var writingRecallPreview = false
    private var recallMatches: [CommandRecall.Entry] = []
    private var selectedRecallIndex = 0
    private struct Completion {
        let text: String
        let cursor: Int
    }

    private var pendingCompletion: (id: String, text: String, selection: NSRange)?
    private var completions: [Completion] = []
    private var selectedCompletion = 0
    private let directoryLabel = NSTextField(labelWithString: "~")
    private let directoryChip = NSView()
    private let divider = NSView()
    private var directory = FileManager.default.homeDirectoryForCurrentUser.path
    private var pendingCommand: String?
    private var activeCommands: [Int: (text: String, directory: String, start: TimeInterval)] = [:]
    private var editorHeight: NSLayoutConstraint?
    private var footerHeight: NSLayoutConstraint?
    private var showingRawHistory = false
    private let editorScroll = CommandEditor.scrollableTextView()
    private let statusLabel = NSTextField(labelWithString: "Starting zsh…")
    private let token = UUID().uuidString
    private var sessionDirectory: URL?
    private var startingShell = true
    private var ready = false
    private var awaitingSubmission = false
    private var manuallyPassthrough = false
    private var closed = false
    private var editor: CommandEditor { editorScroll.documentView as! CommandEditor }

    override init(frame: CGRect) {
        super.init(frame: frame)
        terminal.processDelegate = self
        terminal.font = TerminalAppearance.font
        terminal.nativeBackgroundColor = TerminalAppearance.background
        terminal.nativeForegroundColor = TerminalAppearance.foreground
        editor.font = TerminalAppearance.font
        editor.textColor = TerminalAppearance.foreground
        editor.insertionPointColor = TerminalAppearance.foreground
        editor.drawsBackground = false
        editorScroll.drawsBackground = false
        editorScroll.autohidesScrollers = true
        editor.delegate = self
        editor.isRichText = false
        editor.allowsUndo = true
        editor.isAutomaticQuoteSubstitutionEnabled = false
        editor.isAutomaticDashSubstitutionEnabled = false
        editor.isAutomaticTextReplacementEnabled = false
        editor.isAutomaticSpellingCorrectionEnabled = false
        editor.textContainerInset = NSSize(width: 0, height: 4)
        editor.textContainer?.lineFragmentPadding = 0
        editor.submit = { [weak self] in self?.submit() }
        editor.passthrough = { [weak self] in self?.enterPassthrough() }
        editor.handleRecallKey = { [weak self] event in self?.handleRecallKey(event) ?? false }
        recallList.accept = { [weak self] index in self?.acceptCompletion(at: index) }
        recallList.isHidden = true
        historyPanel.isHidden = true
        historyPanel.preview = { [weak self] row in
            guard let self else { return }
            previewRecall(at: recallMatches.count - 1 - row)
            window?.makeFirstResponder(editor)
        }
        historyPanel.accept = { [weak self] row in
            guard let self else { return }
            acceptRecall(at: recallMatches.count - 1 - row)
        }
        historyPanel.configurationChanged = { [weak self] in self?.refreshRecall(preview: true) }
        editor.setAccessibilityLabel("Command")
        editorScroll.hasVerticalScroller = true
        statusLabel.isHidden = true
        statusLabel.textColor = TerminalAppearance.muted
        statusLabel.font = TerminalAppearance.context
        statusLabel.lineBreakMode = .byTruncatingMiddle

        directoryLabel.font = TerminalAppearance.heading
        directoryLabel.textColor = TerminalAppearance.foreground
        directoryLabel.lineBreakMode = .byTruncatingMiddle
        directoryLabel.translatesAutoresizingMaskIntoConstraints = false
        directoryChip.wantsLayer = true
        directoryChip.layer?.backgroundColor = NSColor(white: 0.065, alpha: 1).cgColor
        directoryChip.layer?.cornerRadius = 4
        directoryChip.layer?.borderWidth = 1
        directoryChip.layer?.borderColor = TerminalAppearance.separator.cgColor
        directoryChip.addSubview(directoryLabel)
        divider.wantsLayer = true
        divider.layer?.backgroundColor = TerminalAppearance.separator.cgColor

        let footer = NSStackView(views: [statusLabel, directoryChip, editorScroll])
        footer.orientation = .vertical
        footer.alignment = .leading
        footer.spacing = 5
        footer.edgeInsets = NSEdgeInsets(top: 9, left: 16, bottom: 12, right: 16)
        for view in [terminal, history, divider, footer, recallList, historyPanel] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        let height = editorScroll.heightAnchor.constraint(equalToConstant: 29)
        editorHeight = height
        let footerHeight = footer.heightAnchor.constraint(equalToConstant: 86)
        self.footerHeight = footerHeight
        let listHeight = recallList.heightAnchor.constraint(equalToConstant: 33)
        recallListHeight = listHeight
        let historyHeight = historyPanel.heightAnchor.constraint(equalToConstant: 100)
        historyPanelHeight = historyHeight
        NSLayoutConstraint.activate([
            historyPanel.leadingAnchor.constraint(equalTo: leadingAnchor),
            historyPanel.trailingAnchor.constraint(equalTo: trailingAnchor),
            historyPanel.bottomAnchor.constraint(equalTo: footer.topAnchor),
            historyHeight,
            recallList.leadingAnchor.constraint(equalTo: editorScroll.leadingAnchor),
            recallList.trailingAnchor.constraint(equalTo: editorScroll.trailingAnchor),
            recallList.bottomAnchor.constraint(equalTo: footer.topAnchor, constant: -4),
            listHeight,
            terminal.topAnchor.constraint(equalTo: topAnchor),
            terminal.bottomAnchor.constraint(equalTo: divider.topAnchor),
            terminal.leadingAnchor.constraint(equalTo: leadingAnchor),
            terminal.trailingAnchor.constraint(equalTo: trailingAnchor),
            history.topAnchor.constraint(equalTo: terminal.topAnchor),
            history.bottomAnchor.constraint(equalTo: terminal.bottomAnchor),
            history.leadingAnchor.constraint(equalTo: terminal.leadingAnchor),
            history.trailingAnchor.constraint(equalTo: terminal.trailingAnchor),
            divider.topAnchor.constraint(equalTo: history.bottomAnchor),
            divider.bottomAnchor.constraint(equalTo: footer.topAnchor),
            divider.leadingAnchor.constraint(equalTo: leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: trailingAnchor),
            divider.heightAnchor.constraint(equalToConstant: 1),
            footer.bottomAnchor.constraint(equalTo: bottomAnchor),
            footerHeight,
            footer.leadingAnchor.constraint(equalTo: leadingAnchor),
            footer.trailingAnchor.constraint(equalTo: trailingAnchor),
            editorScroll.widthAnchor.constraint(equalTo: widthAnchor, constant: -32),
            height,
            statusLabel.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, constant: -32),
            directoryChip.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, constant: -32),
            directoryLabel.leadingAnchor.constraint(equalTo: directoryChip.leadingAnchor, constant: 6),
            directoryLabel.trailingAnchor.constraint(equalTo: directoryChip.trailingAnchor, constant: -6),
            directoryLabel.topAnchor.constraint(equalTo: directoryChip.topAnchor, constant: 2),
            directoryLabel.bottomAnchor.constraint(equalTo: directoryChip.bottomAnchor, constant: -2),
        ])
    }

    required init?(coder: NSCoder) {
        nil
    }

    func start() {
        layoutSubtreeIfNeeded()
        window?.makeFirstResponder(editor)
        var environment = ProcessInfo.processInfo.environment
        environment["TERM"] = "xterm-256color"
        environment["COLORTERM"] = "truecolor"
        if environment["LANG"] == nil && environment["LC_ALL"] == nil {
            environment["LANG"] = "en_US.UTF-8"
        }

        do {
            let directory = try prepareIntegration()
            if let original = environment["ZDOTDIR"] {
                environment["WARP_SWIFT_ORIGINAL_ZDOTDIR"] = original
            }
            environment["ZDOTDIR"] = directory.path
            environment["WARP_SWIFT_SESSION"] = directory.path
            environment["WARP_SWIFT_TOKEN"] = token
        } catch {
            setReady(false)
            statusLabel.stringValue = "Direct terminal · Shell integration unavailable"
        }

        terminal.startProcess(
            executable: "/bin/zsh",
            args: ["-i"],
            environment: environment.map { "\($0.key)=\($0.value)" },
            currentDirectory: FileManager.default.homeDirectoryForCurrentUser.path
        )

        if startingShell {
            DispatchQueue.main.asyncAfter(deadline: .now() + 10) { [weak self] in
                guard let self, startingShell, !closed else { return }
                // Startup scripts can prompt for input before installing the ZLE hooks.
                setReady(false)
                statusLabel.stringValue = "Direct terminal · Waiting for shell integration"
            }
        }
    }

    private func prepareIntegration() throws -> URL {
        guard let script = Bundle.main.url(forResource: "integration", withExtension: "zsh", subdirectory: "Shell") else {
            throw CocoaError(.fileNoSuchFile)
        }
        let directory = FileManager.default.temporaryDirectory.appending(path: "warp-swift-\(token)")
        try FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700]
        )
        do {
            try FileManager.default.copyItem(at: script, to: directory.appending(path: ".zshrc"))
        } catch {
            try? FileManager.default.removeItem(at: directory)
            throw error
        }
        sessionDirectory = directory
        return directory
    }

    func close() {
        guard !closed else { return }
        closed = true
        terminal.processDelegate = nil
        terminal.terminate()
        if let sessionDirectory {
            try? FileManager.default.removeItem(at: sessionDirectory)
        }
    }

    private func submit() {
        guard ready, let sessionDirectory, !editor.string.isEmpty else { return }
        guard !editor.string.utf8.contains(0) else {
            statusLabel.stringValue = "The command contains a NUL character"
            return
        }
        do {
            try Data(editor.string.utf8).write(to: sessionDirectory.appending(path: "command"), options: .atomic)
        } catch {
            statusLabel.stringValue = "Could not queue command: \(error.localizedDescription)"
            return
        }

        awaitingSubmission = true
        pendingCommand = editor.string
        setReady(false)
        window?.makeFirstResponder(terminal)
        statusLabel.stringValue = "Running · Direct terminal input"
        terminal.send(txt: "\u{1b}[98~")
    }

    private func enterPassthrough() {
        manuallyPassthrough = true
        setReady(false)
        window?.makeFirstResponder(terminal)
        statusLabel.stringValue = "Direct terminal · Run a command to return to the editor"
    }

    private func setReady(_ value: Bool) {
        let shouldMoveFocus = window?.firstResponder === terminal || window?.firstResponder === editor
        startingShell = false
        ready = value
        if !value {
            dismissCompletions()
            closeRecall(restoreDraft: false)
            editor.suggestion = nil
        }
        showingRawHistory = false
        editorScroll.isHidden = !value
        directoryChip.isHidden = !value
        divider.isHidden = !value
        statusLabel.isHidden = value
        history.isHidden = !value
        updateEditorHeight()
        if value {
            layoutSubtreeIfNeeded()
            history.scrollToLatest()
            updateSuggestion()
        }
        if shouldMoveFocus {
            window?.makeFirstResponder(value ? editor : terminal)
        }
    }

    func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {}

    func setTerminalTitle(source: LocalProcessTerminalView, title: String) {
        guard !closed else { return }
        let prefix = "warp-swift:\(token):"
        guard title.hasPrefix(prefix) else { return }
        let event = title.dropFirst(prefix.count)
        if event.hasPrefix("completed:") {
            finishCompletion(id: String(event.dropFirst("completed:".count)))
        } else if event == "ready" && !manuallyPassthrough {
            awaitingSubmission = false
            setReady(true)
        } else if event.hasPrefix("busy:"), let pid = Int(event.dropFirst(5)) {
            if awaitingSubmission, let pendingCommand {
                recall.record(pendingCommand, session: token)
                activeCommands[pid] = (pendingCommand, directory, ProcessInfo.processInfo.systemUptime)
                self.pendingCommand = nil
                editor.string = ""
                updateEditorHeight()
                editor.undoManager?.removeAllActions()
                awaitingSubmission = false
            }
            setReady(false)
            manuallyPassthrough = false
            statusLabel.stringValue = "Running · Direct terminal input"
        } else if event.hasPrefix("done:") {
            let fields = event.dropFirst(5).split(separator: ":", maxSplits: 2)
            guard fields.count == 3, let pid = Int(fields[0]), let code = Int(fields[1]) else { return }
            directory = String(fields[2])
            directoryLabel.stringValue = abbreviated(directory)
            window?.title = abbreviated(directory)
            if let command = activeCommands.removeValue(forKey: pid) {
                let marker = "WS\(token.prefix(8))-\(pid)-"
                let snapshot = terminal.terminalContentSnapshot(region: .history(maximumScrollbackRows: 500), buffer: .normal)
                let output = snapshot.flatMap { CommandHistory.output(from: $0, marker: marker) }
                    ?? NSAttributedString(string: "Output is available in the live terminal (⌘F).", attributes: [
                        .font: TerminalAppearance.context, .foregroundColor: TerminalAppearance.muted,
                    ])
                history.append(command: command.text, directory: abbreviated(command.directory),
                               duration: ProcessInfo.processInfo.systemUptime - command.start,
                               exitCode: code, output: output)
            }
        }
    }

    private func abbreviated(_ path: String) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        if path == home { return "~" }
        if path.hasPrefix(home + "/") { return "~" + path.dropFirst(home.count) }
        return path
    }

    func textDidChange(_ notification: Notification) {
        guard !writingRecallPreview else { return }
        dismissCompletions()
        updateEditorHeight()
        if recallDraft != nil {
            recallQuery = (editor.string, editor.selectedRange())
            refreshRecall(preview: false)
        }
        updateSuggestion()
    }

    func textViewDidChangeSelection(_ notification: Notification) {
        updateSuggestion()
    }

    private func handleRecallKey(_ event: NSEvent) -> Bool {
        guard ready else { return false }
        let modifiers = event.modifierFlags.intersection([.command, .control, .option, .shift])
        if modifiers == .control && event.charactersIgnoringModifiers?.lowercased() == "r" {
            openRecall()
            return true
        }
        if recallDraft != nil && !modifiers.isEmpty {
            if modifiers.contains(.command) || event.keyCode == 36 || event.keyCode == 76 {
                acceptRecall(at: selectedRecallIndex)
            } else {
                restoreRecallQuery()
            }
            return false
        }
        guard modifiers.isEmpty else { return false }
        if !completions.isEmpty {
            switch event.keyCode {
            case 48, 125:
                selectedCompletion = (selectedCompletion + 1) % completions.count
                renderCompletions()
                return true
            case 126:
                selectedCompletion = (selectedCompletion + completions.count - 1) % completions.count
                renderCompletions()
                return true
            case 36, 76:
                acceptCompletion(at: selectedCompletion)
                return true
            case 53:
                dismissCompletions()
                updateSuggestion()
                return true
            default:
                dismissCompletions()
            }
        }
        if recallDraft != nil {
            switch event.keyCode {
            case 126, 125:
                let movement = event.keyCode == 126 ? 1 : -1
                let next = previewingRecall ? selectedRecallIndex + movement : 0
                previewRecall(at: min(max(0, recallMatches.count - 1), max(0, next)))
                return true
            case 36, 76:
                acceptRecall(at: selectedRecallIndex)
                return true
            case 53:
                closeRecall(restoreDraft: true)
                return true
            case 123, 124, 115, 119:
                acceptRecall(at: selectedRecallIndex)
                return false
            default:
                restoreRecallQuery()
            }
        } else if event.keyCode == 48 {
            requestCompletion()
            return true
        } else if event.keyCode == 126 && !editor.string.contains("\n") {
            openRecall()
            return true
        } else if event.keyCode == 124, let suggestion = editor.suggestion,
                  editor.selectedRange() == NSRange(location: editor.string.utf16.count, length: 0) {
            replaceEditorText(with: editor.string + suggestion)
            return true
        }
        return false
    }

    private func openRecall() {
        guard recallDraft == nil else { return }
        dismissCompletions()
        recallDraft = (editor.string, editor.selectedRange())
        recallQuery = recallDraft
        editor.suggestion = nil
        historyPanel.isHidden = false
        refreshRecall(preview: true)
    }

    private func refreshRecall(preview: Bool) {
        guard let recallQuery else { return }
        recallMatches = recall.matches(recallQuery.text, session: token)
        selectedRecallIndex = 0
        previewingRecall = false
        historyPanel.show(entries: Array(recallMatches.reversed()))
        if preview && !recallMatches.isEmpty {
            previewRecall(at: 0)
        } else {
            showRecallPanel()
            historyPanel.highlight(nil, scrollToEnd: true)
        }
    }

    private func previewRecall(at index: Int) {
        guard recallMatches.indices.contains(index) else { return }
        selectedRecallIndex = index
        previewingRecall = true
        let command = recallMatches[index].command
        writeRecallPreview(command, selection: NSRange(location: command.utf16.count, length: 0))
        showRecallPanel()
        historyPanel.highlight(recallMatches.count - 1 - index)
    }

    private func showRecallPanel() {
        let desired = CGFloat(min(9, max(1, recallMatches.count)) * 21 + 71)
        let available = max(92, bounds.height - (footerHeight?.constant ?? 86) - 4)
        historyPanelHeight?.constant = min(available, desired)
        layoutSubtreeIfNeeded()
    }

    private func restoreRecallQuery() {
        guard previewingRecall, let recallQuery else { return }
        previewingRecall = false
        writeRecallPreview(recallQuery.text, selection: recallQuery.selection)
        historyPanel.highlight(nil)
    }

    private func writeRecallPreview(_ text: String, selection: NSRange) {
        writingRecallPreview = true
        editor.string = text
        editor.setSelectedRange(selection)
        writingRecallPreview = false
        updateEditorHeight()
    }

    private func acceptRecall(at index: Int) {
        guard recallMatches.indices.contains(index) else { return }
        let command = recallMatches[index].command
        closeRecall(restoreDraft: true)
        if command != editor.string {
            replaceEditorText(with: command)
        }
        window?.makeFirstResponder(editor)
    }

    private func closeRecall(restoreDraft: Bool) {
        guard let draft = recallDraft else { return }
        recallDraft = nil
        recallQuery = nil
        previewingRecall = false
        historyPanel.isHidden = true
        recallMatches = []
        if restoreDraft {
            writeRecallPreview(draft.text, selection: draft.selection)
        }
        updateSuggestion()
    }

    private func replaceEditorText(with text: String, cursor: Int? = nil) {
        let range = NSRange(location: 0, length: editor.string.utf16.count)
        guard editor.shouldChangeText(in: range, replacementString: text) else { return }
        editor.replaceCharacters(in: range, with: text)
        editor.setSelectedRange(NSRange(location: cursor ?? text.utf16.count, length: 0))
        editor.didChangeText()
    }

    private func updateSuggestion() {
        let end = NSRange(location: editor.string.utf16.count, length: 0)
        guard ready, recallDraft == nil, completions.isEmpty,
              editor.selectedRange() == end, !editor.hasMarkedText(),
              !editor.string.contains("\n"),
              let continuation = recall.continuation(after: editor.string, session: token),
              !continuation.contains(where: \.isNewline) else {
            editor.suggestion = nil
            return
        }
        editor.suggestion = continuation
    }

    private func requestCompletion() {
        guard pendingCompletion == nil, let sessionDirectory else { return }
        let draft = editor.string
        let selection = editor.selectedRange()
        guard selection.length == 0, !draft.isEmpty, draft.utf8.count <= 8192,
              !draft.utf8.contains(0) else {
            NSSound.beep()
            return
        }

        let id = UUID().uuidString
        let cursor = String(decoding: draft.utf16.prefix(selection.location), as: UTF16.self)
            .unicodeScalars.count
        let request = Data("\(id)\n\(cursor)\n\(draft)".utf8)
        do {
            try request.write(to: sessionDirectory.appending(path: "completion-request"), options: .atomic)
        } catch {
            NSSound.beep()
            return
        }
        pendingCompletion = (id, draft, selection)
        terminal.send(txt: "\u{1b}[99~")
        DispatchQueue.main.asyncAfter(deadline: .now() + 5) { [weak self] in
            guard let self, pendingCompletion?.id == id else { return }
            pendingCompletion = nil
        }
    }

    private func finishCompletion(id: String) {
        guard UUID(uuidString: id)?.uuidString == id, let sessionDirectory else { return }
        let response = sessionDirectory.appending(path: "completion-\(id)")
        defer { try? FileManager.default.removeItem(at: response) }
        guard let pendingCompletion, pendingCompletion.id == id else { return }
        self.pendingCompletion = nil
        guard ready, recallDraft == nil, window?.firstResponder === editor,
              editor.string == pendingCompletion.text, editor.selectedRange() == pendingCompletion.selection,
              let data = try? Data(contentsOf: response) else { return }

        let fields = data.split(separator: 0, omittingEmptySubsequences: false)
        guard fields.last?.isEmpty == true else { return }
        var candidates: [Completion] = []
        for index in stride(from: 0, to: fields.count - 2, by: 2) {
            guard let cursor = Int(String(decoding: fields[index], as: UTF8.self)),
                  let text = String(data: fields[index + 1], encoding: .utf8),
                  (0...text.unicodeScalars.count).contains(cursor) else { continue }
            let utf16Cursor = String(String.UnicodeScalarView(text.unicodeScalars.prefix(cursor))).utf16.count
            if text != pendingCompletion.text || utf16Cursor != pendingCompletion.selection.location {
                if !candidates.contains(where: { $0.text == text && $0.cursor == utf16Cursor }) {
                    candidates.append(Completion(text: text, cursor: utf16Cursor))
                }
            }
        }
        switch candidates.count {
        case 0:
            NSSound.beep()
        case 1:
            replaceEditorText(with: candidates[0].text, cursor: candidates[0].cursor)
        default:
            let availableHeight = bounds.height - (footerHeight?.constant ?? 86) - 16
            completions = Array(candidates.prefix(max(1, Int(availableHeight / 24))))
            selectedCompletion = 0
            editor.suggestion = nil
            recallList.isHidden = false
            renderCompletions()
        }
    }

    private func renderCompletions() {
        recallList.update(title: "Shell Completions", choices: completions.map(\.text), selected: selectedCompletion)
        recallListHeight?.constant = CGFloat(completions.count * 24 + 9)
    }

    private func acceptCompletion(at index: Int) {
        guard completions.indices.contains(index) else { return }
        let completion = completions[index]
        dismissCompletions()
        replaceEditorText(with: completion.text, cursor: completion.cursor)
        window?.makeFirstResponder(editor)
    }

    private func dismissCompletions() {
        guard !completions.isEmpty else { return }
        completions = []
        recallList.isHidden = true
    }

    func textDidBeginEditing(_ notification: Notification) {
        guard showingRawHistory, ready else { return }
        showingRawHistory = false
        history.isHidden = false
    }

    private func updateEditorHeight() {
        let lines = editor.string.utf8.lazy.filter { $0 == 10 }.prefix(5).count + 1
        let extra = CGFloat(lines - 1) * 17
        editorHeight?.constant = 29 + extra
        footerHeight?.constant = startingShell || ready ? 86 + extra : 38
    }

    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}

    func processFailedToStart(source: TerminalView, error: LocalProcessError) {
        setReady(false)
        statusLabel.stringValue = "Could not start zsh"
    }

    func processTerminated(source: TerminalView, exitCode: Int32?) {
        setReady(false)
        statusLabel.stringValue = exitCode.map { "Shell exited (\($0))" } ?? "Shell terminated"
    }

    func jumpToCommand(previous: Bool) {
        if ready && !showingRawHistory {
            history.jump(previous: previous)
            return
        }
        let row = previous ? terminal.previousSemanticPromptRow() : terminal.nextSemanticPromptRow()
        if let row {
            terminal.scrollTo(row: row)
        }
    }

    func showFind() {
        if ready {
            showingRawHistory = true
            history.isHidden = true
        }
        let item = NSMenuItem()
        item.tag = Int(NSFindPanelAction.showFindPanel.rawValue)
        terminal.performFindPanelAction(item)
    }
}
