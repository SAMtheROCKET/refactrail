'use strict';

// Finding the Python interpreter and installing the tool into it. Shared
// verbatim by the FuncLoom, RefacTrail and FlowBlueprint extensions.

const path = require('node:path');
const { spawn } = require('node:child_process');

const INSTALL_TIMEOUT_MS = 5 * 60 * 1000;

// An explicit setting wins; relative paths resolve from the workspace.
// Otherwise the interpreter selected in the Python extension is used, and
// finally the platform's usual command.
async function resolvePythonPath(vscode, uri, configured) {
    if (configured) {
        if (path.isAbsolute(configured) || !/[\\/]/.test(configured)) { return configured; }
        const workspace = vscode.workspace.getWorkspaceFolder(uri);
        if (!workspace) { throw new Error('Use an absolute Python executable path.'); }
        return path.resolve(workspace.uri.fsPath, configured);
    }
    const selected = await selectedInterpreter(vscode, uri);
    return selected || defaultPython(process.platform);
}

async function selectedInterpreter(vscode, uri) {
    const extension = vscode.extensions && vscode.extensions.getExtension('ms-python.python');
    if (!extension) { return null; }
    try {
        const api = extension.isActive ? extension.exports : await extension.activate();
        const environments = api && api.environments;
        if (!environments || !environments.getActiveEnvironmentPath) { return null; }
        const active = environments.getActiveEnvironmentPath(uri);
        const resolved = environments.resolveEnvironment
            ? await environments.resolveEnvironment(active) : null;
        return (resolved && resolved.executable && resolved.executable.uri
            && resolved.executable.uri.fsPath) || (active && active.path) || null;
    } catch {
        return null;
    }
}

function defaultPython(platform) {
    return platform === 'win32' ? 'python' : 'python3';
}

function installArguments(tool, version) {
    if (!['funcloom', 'refactrail', 'flowblueprint'].includes(tool) || !/^[0-9A-Za-z.]+$/.test(version)) {
        throw new Error('Unsupported tool or version');
    }
    return ['-m', 'pip', 'install', `${tool}==${version}`];
}

function runInstall(pythonPath, tool, version, spawnProcess = spawn) {
    return new Promise((resolve, reject) => {
        const child = spawnProcess(pythonPath, installArguments(tool, version), {
            shell: false,
            windowsHide: true,
            env: { ...process.env, PYTHONUTF8: '1' },
        });
        let output = '';
        const timer = setTimeout(() => { child.kill(); reject(new Error('Installation exceeded 5 minutes')); },
            INSTALL_TIMEOUT_MS);
        child.stdout.setEncoding('utf8');
        child.stderr.setEncoding('utf8');
        child.stdout.on('data', text => { output += text; });
        child.stderr.on('data', text => { output += text; });
        child.on('error', error => { clearTimeout(timer); reject(error); });
        child.on('close', code => { clearTimeout(timer); resolve({ code, output }); });
    });
}

// Make sure the tool runs in the interpreter at the expected version. When
// it does not, ask once, then install (or update) it with pip; nothing is
// installed without the user's click.
async function ensureTool(vscode, options) {
    const { pythonPath, tool, displayName, version, runPython, install = runInstall } = options;
    let problem;
    try {
        const runtime = await runPython(pythonPath, tool, ['--version']);
        if (runtime.code === 0 && runtime.output.includes(version)) { return; }
        problem = runtime.code === 0
            ? `${displayName} ${version} is needed; ${runtime.output.trim()} is installed`
            : `${displayName} is not installed`;
    } catch (error) {
        if (error && error.code === 'ENOENT') {
            await offerInterpreter(vscode, `Python was not found (${pythonPath}). Install Python 3.12 or newer, or choose an interpreter.`);
            throw new Error('No Python interpreter is available.');
        }
        throw error;
    }
    const action = runtimeAction(problem);
    const choice = await vscode.window.showWarningMessage(
        `${problem} for ${pythonPath}.`, action, 'Choose interpreter');
    if (choice === 'Choose interpreter') {
        await vscode.commands.executeCommand('python.setInterpreter');
        throw new Error('Run the command again after choosing the interpreter.');
    }
    if (choice !== action) { throw new Error(`${displayName} is not available.`); }
    const result = await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: `Installing ${displayName} ${version}...` },
        () => install(pythonPath, tool, version));
    if (result.code !== 0) {
        throw new Error(`Installation failed. ${lastLines(result.output)} `
            + 'If pip refuses to change this Python, choose a virtual environment.');
    }
    const check = await runPython(pythonPath, tool, ['--version']);
    if (check.code !== 0 || !check.output.includes(version)) {
        throw new Error(`${displayName} was installed but does not run: ${check.errors || check.output}`);
    }
    vscode.window.showInformationMessage(`${displayName} ${version} is ready.`);
}

function runtimeAction(problem) {
    return problem.endsWith('is not installed') ? 'Install' : 'Update';
}

async function offerInterpreter(vscode, message) {
    const choice = await vscode.window.showErrorMessage(message, 'Choose interpreter');
    if (choice) { await vscode.commands.executeCommand('python.setInterpreter'); }
}

function lastLines(text, count = 3) {
    return text.trim().split(/\r?\n/).slice(-count).join(' ');
}

module.exports = {
    resolvePythonPath, defaultPython, installArguments, runInstall, ensureTool, runtimeAction,
};
