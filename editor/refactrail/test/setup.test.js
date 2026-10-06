'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const path = require('node:path');
const {
    resolvePythonPath, defaultPython, installArguments, runInstall, ensureTool,
} = require('../setup');

function fakeVscode({ answer = undefined, pythonApi = null, workspace = null } = {}) {
    const calls = { warnings: [], errors: [], infos: [], commands: [] };
    const vscode = {
        calls,
        workspace: { getWorkspaceFolder: () => workspace },
        extensions: {
            getExtension: id => (id === 'ms-python.python' && pythonApi
                ? { isActive: true, exports: pythonApi } : undefined),
        },
        window: {
            showWarningMessage: async (message, ...actions) => { calls.warnings.push({ message, actions }); return answer; },
            showErrorMessage: async (message, ...actions) => { calls.errors.push({ message, actions }); return undefined; },
            showInformationMessage: message => { calls.infos.push(message); },
            withProgress: async (options, work) => work(),
        },
        commands: { executeCommand: async command => { calls.commands.push(command); } },
        ProgressLocation: { Notification: 15 },
    };
    return vscode;
}

test('install arguments pin the exact tool version', () => {
    assert.deepEqual(installArguments('refactrail', '0.3.1a0'),
        ['-m', 'pip', 'install', 'refactrail==0.3.1a0']);
    assert.deepEqual(installArguments('flowblueprint', '0.2.0a0'),
        ['-m', 'pip', 'install', 'flowblueprint==0.2.0a0']);
    assert.throws(() => installArguments('other', '1.0'), /Unsupported/);
    assert.throws(() => installArguments('funcloom', '1.0; rm -rf /'), /Unsupported/);
});

test('an explicit interpreter setting wins; relative paths use the workspace', async () => {
    const absolute = path.resolve('/opt/python/bin/python3');
    assert.equal(await resolvePythonPath(fakeVscode(), null, absolute), absolute);
    assert.equal(await resolvePythonPath(fakeVscode(), null, 'python3.12'), 'python3.12');
    const workspace = { uri: { fsPath: path.resolve('/work/project') } };
    assert.equal(await resolvePythonPath(fakeVscode({ workspace }), null, '.venv/bin/python'),
        path.resolve('/work/project', '.venv/bin/python'));
    await assert.rejects(resolvePythonPath(fakeVscode(), null, '.venv/bin/python'), /absolute/);
});

test('without a setting the Python extension selection is used', async () => {
    const pythonApi = { environments: {
        getActiveEnvironmentPath: () => ({ path: '/envs/project' }),
        resolveEnvironment: async () => ({ executable: { uri: { fsPath: '/envs/project/bin/python' } } }),
    } };
    assert.equal(await resolvePythonPath(fakeVscode({ pythonApi }), null, ''), '/envs/project/bin/python');
    assert.equal(await resolvePythonPath(fakeVscode(), null, ''), defaultPython(process.platform));
    assert.equal(defaultPython('win32'), 'python');
    assert.equal(defaultPython('linux'), 'python3');
});

test('an installed tool at the right version needs no prompt', async () => {
    const vscode = fakeVscode();
    await ensureTool(vscode, {
        pythonPath: 'python', tool: 'refactrail', displayName: 'RefacTrail', version: '0.3.1a0',
        runPython: async () => ({ code: 0, output: 'refactrail 0.3.1a0\n', errors: '' }),
        install: async () => { throw new Error('must not install'); },
    });
    assert.equal(vscode.calls.warnings.length, 0);
});

test('a missing tool is installed only after the user clicks Install', async () => {
    let installed = null;
    let runs = 0;
    const runPython = async () => (++runs === 1
        ? { code: 1, output: '', errors: 'No module named refactrail' }
        : { code: 0, output: 'refactrail 0.3.1a0', errors: '' });
    const vscode = fakeVscode({ answer: 'Install' });
    await ensureTool(vscode, {
        pythonPath: 'py', tool: 'refactrail', displayName: 'RefacTrail', version: '0.3.1a0', runPython,
        install: async (...args) => { installed = args; return { code: 0, output: 'ok' }; },
    });
    assert.deepEqual(installed, ['py', 'refactrail', '0.3.1a0']);
    assert.deepEqual(vscode.calls.warnings[0].actions, ['Install', 'Choose interpreter']);
    assert.match(vscode.calls.infos[0], /ready/);

    const declined = fakeVscode({ answer: undefined });
    await assert.rejects(ensureTool(declined, {
        pythonPath: 'py', tool: 'refactrail', displayName: 'RefacTrail', version: '0.3.1a0',
        runPython: async () => ({ code: 1, output: '', errors: '' }),
        install: async () => { throw new Error('must not install'); },
    }), /not available/);
});

test('an outdated tool offers Update, and failures explain themselves', async () => {
    const vscode = fakeVscode({ answer: 'Update' });
    await assert.rejects(ensureTool(vscode, {
        pythonPath: 'py', tool: 'funcloom', displayName: 'FuncLoom', version: '0.10.3a0',
        runPython: async () => ({ code: 0, output: 'funcloom 0.10.1a0', errors: '' }),
        install: async () => ({ code: 1, output: 'error: externally-managed-environment' }),
    }), /externally-managed-environment.*virtual environment/);
    assert.deepEqual(vscode.calls.warnings[0].actions, ['Update', 'Choose interpreter']);
});

test('a missing interpreter offers to choose one', async () => {
    const vscode = fakeVscode();
    const missing = Object.assign(new Error('spawn python ENOENT'), { code: 'ENOENT' });
    await assert.rejects(ensureTool(vscode, {
        pythonPath: 'python', tool: 'funcloom', displayName: 'FuncLoom', version: '0.10.3a0',
        runPython: async () => { throw missing; },
    }), /No Python interpreter/);
    assert.match(vscode.calls.errors[0].message, /Python was not found/);
});

test('installation runs pip with an argument array and no shell', async () => {
    let actual;
    const result = await runInstall('C:\\python path\\python.exe', 'funcloom', '0.10.3a0',
        (executable, args, options) => {
            actual = { executable, args, options };
            const child = new EventEmitter();
            child.stdout = new EventEmitter(); child.stdout.setEncoding = () => {};
            child.stderr = new EventEmitter(); child.stderr.setEncoding = () => {};
            child.kill = () => {};
            setImmediate(() => { child.stdout.emit('data', 'Successfully installed'); child.emit('close', 0); });
            return child;
        });
    assert.equal(actual.executable, 'C:\\python path\\python.exe');
    assert.deepEqual(actual.args, ['-m', 'pip', 'install', 'funcloom==0.10.3a0']);
    assert.equal(actual.options.shell, false);
    assert.deepEqual(result, { code: 0, output: 'Successfully installed' });
});
