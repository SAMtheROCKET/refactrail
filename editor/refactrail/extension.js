'use strict';

const path = require('node:path');
const { runPython, ensureTrusted, checkArguments, generalArguments, getUtf16Column, isSupportedInput } = require('./runner');
const { resolvePythonPath, ensureTool } = require('./setup');
const manifest = require('./package.json');

function activate(context) {
    const vscode = require('vscode');
    const tool = manifest.name;
    const output = vscode.window.createOutputChannel(manifest.displayName);
    const diagnostics = vscode.languages.createDiagnosticCollection(tool);
    context.subscriptions.push(output, diagnostics);

    async function execute(command) {
        ensureTrusted(vscode);
        const editor = vscode.window.activeTextEditor;
        const notebook = vscode.window.activeNotebookEditor;
        const uri = editor?.document.uri.scheme === 'vscode-notebook-cell'
            ? notebook?.notebook.uri : (editor?.document.uri || notebook?.notebook.uri);
        if (!uri) { throw new Error('Open a Python file or notebook first.'); }
        const config = vscode.workspace.getConfiguration(tool, uri);
        const pythonPath = await resolvePythonPath(vscode, uri, config.get('pythonPath', ''));
        await ensureTool(vscode, {
            pythonPath, tool, displayName: manifest.displayName,
            version: manifest.toolVersion, runPython,
        });
        let args;
        let input = '';
        if (command === 'snippet') {
            if (!editor || !['python', 'python3'].includes(editor.document.languageId)) {
                throw new Error('Select Python text or open a Python notebook cell.');
            }
            input = editor.selection.isEmpty ? editor.document.getText()
                : editor.document.getText(editor.selection);
            args = ['snippet', '-', '--format', 'json'];
            const choice = await vscode.window.showQuickPick(
                ['No context', 'Use context TOML'], { title: 'FuncLoom context' });
            if (!choice) { return; }
            if (choice === 'Use context TOML') {
                const selected = await vscode.window.showOpenDialog({
                    canSelectMany: false, filters: { 'Context TOML': ['toml'] },
                });
                if (!selected) { return; }
                args.push('--context', selected[0].fsPath);
            } else { args.push('--no-context'); }
        } else {
            if (uri.scheme !== 'file') { throw new Error('This command requires a local saved file.'); }
            if (editor?.document.isDirty || notebook?.notebook.isDirty) {
                throw new Error('Save the file before running this command.');
            }
            if (!isSupportedInput(command, uri.fsPath)) {
                throw new Error('Use a saved .py or .pyi file. Python notebooks support independent formatting.');
            }
            if (command === 'check') {
                args = checkArguments(tool, uri.fsPath, config.get('profile', 'strict'));
            } else if (['lint', 'formatPreview', 'formatWrite', 'scope'].includes(command)) {
                const formatting = ['formatPreview', 'formatWrite'].includes(command);
                args = generalArguments(command, uri.fsPath, config.get('lineLength', 79),
                    formatting ? config.get('bracketStyle', 'own-line') : null);
            } else if (command === 'index') {
                const workspace = vscode.workspace.getWorkspaceFolder(uri);
                if (!workspace) { throw new Error('Open a project workspace before indexing.'); }
                args = generalArguments('index', path.resolve(workspace.uri.fsPath, config.get('importRoot', '.')));
            } else if (command === 'rename') {
                const functionName = await vscode.window.showInputBox({ title: 'Top-level function containing the local variable' });
                if (!functionName) { return; }
                const oldName = await vscode.window.showInputBox({ title: 'Existing local variable name' });
                if (!oldName) { return; }
                const newName = await vscode.window.showInputBox({ title: 'Proposed meaningful variable name' });
                if (!newName) { return; }
                args = ['rename', uri.fsPath, '--function', functionName, '--old', oldName, '--new', newName];
            } else if (command === 'preview' || command === 'fix') {
                args = ['fix', uri.fsPath, '--profile', config.get('profile', 'strict')];
                if (command === 'preview') { args.push('--diff'); }
            } else {
                const outputUri = await vscode.window.showSaveDialog({
                    title: 'Choose a new output file or folder name',
                    defaultUri: vscode.Uri.file(uri.fsPath + (command === 'refine' ? '.refined.py' : '.modular')),
                });
                if (!outputUri) { return; }
                args = [command, uri.fsPath, '--output', outputUri.fsPath, '--format', 'json'];
            }
        }
        const result = await runPython(pythonPath, tool, args, input);
        output.clear();
        output.appendLine(result.output);
        if (result.errors) { output.appendLine(result.errors); }
        output.show(true);
        if (result.code === null || result.code > 1) {
            throw new Error(result.errors || result.output || 'The tool failed.');
        }
        if (command === 'check' || command === 'lint') {
            const report = JSON.parse(result.output);
            const findings = Array.isArray(report) ? report : [
                ...(report.diagnostics || []),
                ...(report.modules || []).flatMap(module => module.diagnostics || []),
            ];
            diagnostics.set(uri, findings.map(finding => {
                const line = Math.max(0, finding.line - 1);
                const lineText = editor && line < editor.document.lineCount
                    ? editor.document.lineAt(line).text : '';
                const column = getUtf16Column(lineText, finding.column);
                const endColumn = Math.max(column + 1,
                    getUtf16Column(lineText, finding.column + 1));
                const diagnostic = new vscode.Diagnostic(
                    new vscode.Range(line, column, line, endColumn), finding.message,
                    finding.severity === 'error' ? vscode.DiagnosticSeverity.Error : vscode.DiagnosticSeverity.Warning);
                diagnostic.code = finding.code;
                diagnostic.source = tool;
                return diagnostic;
            }));
        } else if (!['fix', 'formatWrite'].includes(command)) {
            const document = await vscode.workspace.openTextDocument({
                content: result.output, language: ['preview', 'formatPreview'].includes(command) ? 'diff' : 'json',
            });
            await vscode.window.showTextDocument(document, { preview: true });
        }
    }

    for (const contribution of manifest.contributes.commands) {
        const command = contribution.command.split('.').at(-1);
        context.subscriptions.push(vscode.commands.registerCommand(contribution.command,
            () => execute(command).catch(error => vscode.window.showErrorMessage(error.message))));
    }
}

module.exports = { activate };
