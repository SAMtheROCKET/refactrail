'use strict';

// End-to-end editor workflows for RefacTrail, run inside a VS Code
// extension host. Prompts are answered by stubbing vscode.window, so the
// run needs no clicks. Results go to host-workflows.json in the workspace.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vscode = require('vscode');
const manifest = require('../package.json');

const TOOL = manifest.name;

function stubWindow(stubs) {
    const originals = {};
    for (const [name, value] of Object.entries(stubs)) {
        originals[name] = vscode.window[name];
        vscode.window[name] = value;
    }
    return () => Object.assign(vscode.window, originals);
}

async function runCommand(command, stubs = {}) {
    const errors = [];
    const restore = stubWindow({
        ...stubs,
        showErrorMessage: async message => { errors.push(message); },
    });
    try {
        await vscode.commands.executeCommand(`${TOOL}.${command}`);
    } finally {
        restore();
    }
    return errors;
}

async function openFile(filePath) {
    const document = await vscode.workspace.openTextDocument(vscode.Uri.file(filePath));
    return vscode.window.showTextDocument(document);
}

function activeDocument() {
    return vscode.window.activeTextEditor?.document;
}

function problems(filePath) {
    return vscode.languages.getDiagnostics(vscode.Uri.file(filePath))
        .filter(item => item.source === TOOL);
}

async function run() {
    const root = vscode.workspace.workspaceFolders[0].uri.fsPath;
    const results = {};
    const step = async (name, action) => {
        try {
            results[name] = (await action()) || 'passed';
        } catch (error) {
            results[name] = `FAILED: ${error.message}`;
            fs.writeFileSync(path.join(root, 'host-workflows.json'), JSON.stringify(results, null, 2));
            throw error;
        }
    };
    await step('activation and command registration', async () => {
        await vscode.extensions.getExtension(`samtherocket.${TOOL}`).activate();
        const commands = await vscode.commands.getCommands(true);
        for (const command of manifest.contributes.commands) {
            assert.ok(commands.includes(command.command), command.command);
        }
    });
    const config = vscode.workspace.getConfiguration(TOOL);
    await config.update('pythonPath', process.env.PYTHON_REVIEW_PATH, vscode.ConfigurationTarget.Workspace);

    const fixture = path.join(root, 'invoice.py');
    const fixtureText = '"""Invoice totals."""\n\n\ndef compute_total(amount, items=[]):\n'
        + '    """Add tax.\n\n    Args:\n        amount: Base amount.\n        items: Lines.\n'
        + '    Returns:\n        Total.\n    """\n    total=amount*1.1\n    return total\n';
    fs.writeFileSync(fixture, fixtureText);
    await step('check populates Problems with strict findings', async () => {
        await openFile(fixture);
        assert.deepEqual(await runCommand('check'), []);
        const found = problems(fixture);
        assert.ok(found.some(item => String(item.code).startsWith('RT')), 'RT findings expected');
        assert.equal(fs.readFileSync(fixture, 'utf8'), fixtureText);
        return `passed (${found.length} problem(s))`;
    });
    await step('lint reports correctness findings in Problems', async () => {
        await openFile(fixture);
        assert.deepEqual(await runCommand('lint'), []);
        assert.ok(problems(fixture).some(item => item.code === 'RC101'), 'mutable default expected');
    });
    await step('fix preview opens a diff and leaves the file alone', async () => {
        await openFile(fixture);
        assert.deepEqual(await runCommand('preview'), []);
        assert.equal(activeDocument().languageId, 'diff');
        assert.equal(fs.readFileSync(fixture, 'utf8'), fixtureText);
    });
    await step('format preview shows the spacing diff', async () => {
        await openFile(fixture);
        assert.deepEqual(await runCommand('formatPreview'), []);
        assert.equal(activeDocument().languageId, 'diff');
        assert.ok(activeDocument().getText().includes('total = amount * 1.1'));
    });
    await step('scope opens a JSON scope report', async () => {
        await openFile(fixture);
        assert.deepEqual(await runCommand('scope'), []);
        assert.equal(JSON.parse(activeDocument().getText()).schema_version, 'refactrail-scope-1');
    });
    await step('index opens a JSON project index', async () => {
        await openFile(fixture);
        assert.deepEqual(await runCommand('index'), []);
        assert.equal(JSON.parse(activeDocument().getText()).schema_version, 'refactrail-index-1');
    });
    await step('rename asks three questions and proposes, never applies', async () => {
        await openFile(fixture);
        const answers = ['compute_total', 'total', 'total_amount_float'];
        const errors = await runCommand('rename', { showInputBox: async () => answers.shift() });
        assert.deepEqual(errors, []);
        assert.ok(activeDocument().getText().includes('total_amount_float'));
        assert.equal(fs.readFileSync(fixture, 'utf8'), fixtureText);
    });
    await step('rename cancelled at the first question does nothing', async () => {
        const editor = await openFile(fixture);
        assert.deepEqual(await runCommand('rename', { showInputBox: async () => undefined }), []);
        assert.equal(activeDocument().uri.fsPath, editor.document.uri.fsPath);
    });
    await step('format write and fix change the saved file', async () => {
        await openFile(fixture);
        assert.deepEqual(await runCommand('formatWrite'), []);
        assert.ok(fs.readFileSync(fixture, 'utf8').includes('total = amount * 1.1'));
        await openFile(fixture);
        assert.deepEqual(await runCommand('fix'), []);
        return 'passed';
    });
    const notebook = path.join(root, 'analysis.ipynb');
    fs.writeFileSync(notebook, JSON.stringify({ nbformat: 4, nbformat_minor: 5,
        metadata: { kernelspec: { language: 'python', name: 'python3', display_name: 'Python 3' } },
        cells: [{ cell_type: 'code', metadata: {}, execution_count: null, outputs: [],
            source: ['base_amount=120\n', 'print(base_amount*2)\n'] }] }));
    await step('notebook formatting preview from the notebook editor', async () => {
        await vscode.commands.executeCommand('workbench.action.closeAllEditors');
        await vscode.window.showNotebookDocument(
            await vscode.workspace.openNotebookDocument(vscode.Uri.file(notebook)));
        assert.deepEqual(await runCommand('formatPreview'), []);
        assert.ok(activeDocument().getText().includes('base_amount = 120'));
    });
    const broken = path.join(root, 'broken.py');
    fs.writeFileSync(broken, 'def broken(:\n    pass\n');
    await step('a file with a syntax error is reported, not hidden', async () => {
        await openFile(broken);
        assert.deepEqual(await runCommand('check'), []);
        assert.ok(problems(broken).some(item => item.code === 'RT001'), 'RT001 expected');
        const errors = await runCommand('formatWrite');
        assert.equal(errors.length, 1, 'formatting must refuse with a message');
        assert.equal(fs.readFileSync(broken, 'utf8'), 'def broken(:\n    pass\n');
        return `passed ("${errors[0].split('\n')[0].slice(0, 80)}")`;
    });
    const notes = path.join(root, 'notes.txt');
    fs.writeFileSync(notes, 'plain text\n');
    await step('unsupported file types get a clear message', async () => {
        await openFile(notes);
        const errors = await runCommand('check');
        assert.equal(errors.length, 1);
        assert.ok(errors[0].startsWith('Use a saved .py or .pyi file'), errors[0]);
    });
    await step('unsaved changes are refused with a message', async () => {
        const editor = await openFile(fixture);
        await editor.edit(builder => builder.insert(new vscode.Position(0, 0), '# edit\n'));
        assert.deepEqual(await runCommand('lint'), ['Save the file before running this command.']);
        await vscode.commands.executeCommand('workbench.action.files.revert');
    });
    await step('wrong interpreter gives an install message', async () => {
        await openFile(fixture);
        await config.update('pythonPath', process.execPath, vscode.ConfigurationTarget.Workspace);
        const errors = await runCommand('check');
        await config.update('pythonPath', process.env.PYTHON_REVIEW_PATH, vscode.ConfigurationTarget.Workspace);
        assert.equal(errors.length, 1);
        assert.ok(errors[0].startsWith(`Install ${TOOL} ${manifest.toolVersion}`), errors[0]);
    });
    fs.writeFileSync(path.join(root, 'host-workflows.json'), JSON.stringify(results, null, 2));
}

module.exports = { run };
