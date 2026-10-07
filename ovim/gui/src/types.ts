export interface GuiKeyInput {
    key: string;
    shift: boolean;
    control: boolean;
    alt: boolean;
    meta: boolean;
}

export interface GuiSegment {
    text: string;
    cells: number;
    token?: string;
    cursor: boolean;
    selected: boolean;
    searchMatch: boolean;
}

export interface GuiLine {
    number: number;
    continuation: boolean;
    displayStart: number;
    current: boolean;
    segments: GuiSegment[];
    git?: "added" | "modified" | "removed";
    diagnostic?: "error" | "warning" | "information" | "hint";
    diff?: "header" | "hunk" | "added" | "removed" | "context";
    breakpoint?: "enabled" | "conditional" | "disabled";
    executing?: boolean;
    /** Lines hidden below this one by a closed fold. */
    folded?: number;
    /** Fold gutter mark: a fold header (open or closed) or a line inside one. */
    fold?: "open" | "closed" | "inside";
}

export type GuiLayoutNode =
    | { kind: "pane"; pane: number }
    | {
          kind: "split";
          direction: "horizontal" | "vertical";
          ratio: number;
          first: GuiLayoutNode;
          second: GuiLayoutNode;
      };

export interface GuiMarkdownHighlight {
    start: number;
    end: number;
    token: string;
}

export interface GuiMarkdownDocument {
    text: string;
    viewLines: number[];
    highlights?: GuiMarkdownHighlight[][];
}

export interface GuiDiffLine {
    kind: "context" | "added" | "removed";
    text: string;
    oldLine?: number;
    newLine?: number;
    reviewLine?: number;
    highlights?: GuiMarkdownHighlight[];
}

export interface GuiDiffContext {
    id: string;
    before: {
        old: { number: number; text: string }[];
        new: { number: number; text: string }[];
    };
    after: GuiDiffContext["before"];
    canExpandUp: boolean;
    canExpandDown: boolean;
}

export interface GuiDiffHunk {
    context?: GuiDiffContext;
    header: string;
    oldStart: number;
    oldCount: number;
    newStart: number;
    newCount: number;
    reviewLine?: number;
    lines: GuiDiffLine[];
}

export interface GuiDiffMoveEndpoint {
    path: string;
    startLine: number;
    lineCount: number;
    reviewLine?: number;
    contextWindows: Array<{
        startLine: number;
        lines: GuiDiffLine[];
    }>;
    contextComplete: boolean;
}

export interface GuiDiffMove {
    id: string;
    label?: string;
    old: GuiDiffMoveEndpoint;
    new: GuiDiffMoveEndpoint;
}

export interface GuiDiffDocument {
    title: string;
    provenance?: {
        baseLabel: string;
        comparisonBaseOid: string;
        snapshotId?: string;
    };
    layout: "split" | "unified";
    managed: boolean;
    custom: boolean;
    showChecked?: boolean;
    moves?: GuiDiffMove[];
    guidedFiles?: GuiDiffDocument["files"];
    overlay?: {
        mode: "active" | "available" | "stale" | "saved";
        title?: string;
    };
    files: Array<{
        id: string;
        checked?: boolean;
        label?: string;
        message?: string;
        path: string;
        oldPath?: string;
        status: string;
        additions: number;
        deletions: number;
        binary: boolean;
        metadata: string[];
        hunks: GuiDiffHunk[];
    }>;
}

export interface GuiPane {
    index: number;
    bufferId: number;
    focused: boolean;
    fileName: string;
    modified: boolean;
    cursor: { line: number; column: number; displayColumn: number };
    firstLine: number;
    scrollSubrow: number;
    horizontalOffset: number;
    totalLines: number;
    lines: GuiLine[];
    markdown?: GuiMarkdownDocument | null;
    diffReview?: GuiDiffDocument | null;
}

export interface GuiAiProfileOption {
    label?: string;
    id: string;
    provider: string;
    model: string;
}

export interface GuiAiChat {
    externalAgent?: boolean;
    externalQuestion?: boolean;
    profile: string;
    model?: string;
    pendingCodeAttachment?: {
        bufferId: number;
        label: string;
        startLine: number;
        startColumn: number;
        endLine: number;
        endColumn: number;
        linewise: boolean;
    };
    profiles: GuiAiProfileOption[];
    reasoningEffort: string;
    reasoningEffortSelection: string;
    reasoningEffortDefault?: string;
    reasoningEfforts: string[];
    permissionMode?: string;
    permissionModes?: Array<{
        id: string;
        label: string;
        description: string;
    }>;
    yoloMode: boolean;
    shellAllowed?: boolean;
    comprehensionPolicy: "off" | "publish" | "commit";
    comprehensionCheckpoint?: string;
    activity: string;
    waiting: boolean;
    input: string;
    inputCursor: number;
    pendingImages: string[];
    queuedInputs: Array<{
        id: number;
        kind: "steer" | "followUp" | "command";
        content: string;
        imageCount: number;
        hasCodeAttachment: boolean;
        selected: boolean;
    }>;
    setup?: {
        kind: string;
        title: string;
        detail: string;
        maskedInput?: string;
        inputCursor?: number;
        error?: string;
        actions: Array<{ label: string; key: string }>;
    };
    messages: Array<{
        id: string;
        index: number;
        selected: boolean;
        role: string;
        content: string;
        attachment?: string;
        model?: string;
        toolName?: string;
        replayToolCallId?: string;
        replayLabel?: string;
        tools: string[];
        images?: string[];
    }>;
    streaming?: string;
    streamingThinking?: string;
    thinkingLive: boolean;
    focus: "textInput" | "messageHistory" | "modelSelector" | "treePanel";
    agents: Array<{
        id: string;
        taskName: string;
        lifecycle: string;
        model: string;
        depth: number;
    }>;
    selectedAgentId?: string;
    followedAgentId?: string;
    agentCursor: number;
    approval?: string;
    codeExplanation?: GuiCodeExplanation;
}

export interface GuiCodeExplanation {
    answerInProgress: boolean;
    current: number;
    total: number;
    page:
        | { kind: "concept"; title: string; body: string }
        | {
              kind: "code";
              path: string;
              startLine: number;
              endLine: number;
              comment: string;
          }
        | {
              kind: "diff";
              title: string;
              oldPath?: string;
              newPath?: string;
              comment: string;
          };
    discussion:
        | {
              state: "navigating";
              questionCount: number;
              latestQuestion?: string;
              latestAnswer?: string;
              latestFailed: boolean;
          }
        | {
              state: "composing";
              input: string;
              cursor: number;
              questionCount: number;
          }
        | {
              state: "answering";
              question: string;
              answer: string;
              questionCount: number;
          };
}

export interface GuiTestPanel {
    scope: string;
    command: string;
    directory: string;
    status: string;
    elapsedMs: number;
    summary?: string;
    truncated: number;
    lines: string[];
}

export interface GuiProblemList {
    kind: string;
    title: string;
    selected: number;
    total: number;
    items: Array<{
        index: number;
        severity: string;
        file: string;
        line: number;
        column: number;
        message: string;
    }>;
}

export interface GuiSearchReplaceRow {
    index: number;
    kind: "file" | "match";
    state: "checked" | "unchecked" | "partial";
    path: string;
    line: number;
    count: number;
    before: string;
    matched: string;
    replacement: string;
    after: string;
}

export interface GuiSearchReplace {
    find: string;
    replace: string;
    files: string;
    focus: "find" | "replace" | "files" | "results";
    regex: boolean;
    caseSensitive: boolean;
    wholeWord: boolean;
    searching: boolean;
    searched: boolean;
    truncated: boolean;
    error?: string;
    totalMatches: number;
    checkedMatches: number;
    fileCount: number;
    selected: number;
    rows: GuiSearchReplaceRow[];
}

export interface GuiLspManager {
    filter: string;
    selected: number;
    showDetail: boolean;
    items: Array<{
        index: number;
        language: string;
        section: string;
        command?: string;
        state?: string;
        installing?: string;
        installHint?: string;
        extensions: string[];
        rootMarkers: string[];
        capabilities: string[];
    }>;
}

export interface GuiDebugPanel {
    running: boolean;
    reason?: string;
    executionLine?: number;
    stack: Array<{
        name: string;
        file: string;
        line: number;
        selected: boolean;
    }>;
    rows: GuiDebugRow[];
    output: string[];
}

export interface GuiDebugRow {
    index: number;
    kind:
        | "header"
        | "note"
        | "variable"
        | "watch"
        | "breakpoint"
        | "exception"
        | "thread";
    depth: number;
    label: string;
    value?: string;
    typeName?: string;
    expandable: boolean;
    expanded: boolean;
    enabled?: boolean;
    conditional: boolean;
    /** Thread rows: the thread whose stack and variables are shown. */
    selected?: boolean;
}

export type GuiRunLineKind =
    "stdout" | "stderr" | "build" | "system" | "debugger";

export interface GuiRunConsole {
    title: string;
    mode: "run" | "debug";
    status: "running" | "succeeded" | "failed" | "stopped" | "error";
    statusText: string;
    active: boolean;
    command: string;
    exitCode?: number;
    elapsedMs: number;
    runIndex: number;
    runCount: number;
    truncated: number;
    firstIndex: number;
    lines: Array<{
        kind: GuiRunLineKind;
        text: string;
        jumpable: boolean;
    }>;
}

export interface GuiTheme {
    name: string;
    background: string;
    foreground: string;
    surface: string;
    surfaceSelected: string;
    border: string;
    accent: string;
    accentForeground: string;
    muted: string;
    cursorLine: string;
    selection: string;
    search: string;
    error: string;
    warning: string;
    info: string;
    success: string;
    syntax: Record<string, string>;
}

export interface GuiSnapshot {
    revision: number;
    mode: string;
    dashboard: boolean;
    filePath?: string;
    fileName: string;
    workspacePath?: string;
    projectName: string;
    language: string;
    encoding: string;
    lineEnding: string;
    modified: boolean;
    hasUnsavedChanges: boolean;
    bufferRevision: number;
    readOnly: boolean;
    selectionText?: string;
    cursor: { line: number; column: number; displayColumn: number };
    horizontalOffset: number;
    wrap: boolean;
    tabWidth: number;
    expandTab: boolean;
    firstLine: number;
    totalLines: number;
    lines: GuiLine[];
    layout: GuiLayoutNode;
    panes: GuiPane[];
    tabs: Array<{
        id: number;
        index: number;
        title: string;
        active: boolean;
        modified: boolean;
    }>;
    gitBranch?: string;
    symbolBreadcrumbs: Array<{ name: string; kind: string }>;
    gitChanges: { added: number; modified: number; removed: number };
    diagnostics: {
        errors: number;
        warnings: number;
        information: number;
        hints: number;
    };
    lspStatus: string;
    statusMessage: string;
    prompt?: { prefix: string; text: string; cursor: number };
    picker?: {
        title: string;
        query: string;
        fileFilter?: string;
        selected: number;
        total: number;
        items: Array<{
            index: number;
            display: string;
            location: string;
            detail?: string;
            matched: number[];
        }>;
    };
    completion?: {
        selected: number;
        /** Number of matching items; `items` is the visible window. */
        total?: number;
        items: Array<{
            index: number;
            label: string;
            /** Text right after the label (labelDetails.detail). */
            detail?: string;
            /** Right-aligned dimmed text (labelDetails.description). */
            description?: string;
            kind?: string;
            kindGlyph?: string;
            kindClass?: string;
            deprecated?: boolean;
            /** Char positions of `label` matched by what was typed. */
            matched?: number[];
        }>;
        /** Markdown: signature and documentation of the selected item. */
        documentation?: string;
    };
    hover?: { content: string; line?: number; displayColumn?: number };
    signatureHelp?: {
        before: string;
        active: string;
        after: string;
        documentation?: string;
        signatureIndex: number;
        signatureCount: number;
        line: number;
        displayColumn: number;
    };
    fileTree?: {
        revealGeneration: number;
        root: string;
        selected: number;
        items: Array<{
            index: number;
            name: string;
            path: string;
            depth: number;
            directory: boolean;
            expanded: boolean;
        }>;
    };
    aiChat?: GuiAiChat;
    testPanel?: GuiTestPanel;
    problems?: GuiProblemList;
    lspManager?: GuiLspManager;
    searchReplace?: GuiSearchReplace;
    debug?: GuiDebugPanel;
    runConsole?: GuiRunConsole;
    theme: GuiTheme;
    shouldQuit: boolean;
}
