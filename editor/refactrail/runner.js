'use strict';

const { spawn } = require('node:child_process');

function runPython(pythonPath, tool, argumentsList, input = '', spawnProcess = spawn) {
    if (!['funcloom', 'refactrail'].includes(tool)) {
        throw new Error('Unsupported tool');
    }
    return new Promise((resolve, reject) => {
        const process = spawnProcess(pythonPath, ['-I', '-m', tool, ...argumentsList], {
            shell: false,
            windowsHide: true,
            env: { ...global.process.env, PYTHONNOUSERSITE: '1', PYTHONUTF8: '1' },
        });
        let output = '';
        let errors = '';
        let settled = false;
        const timer = setTimeout(() => finish(new Error('Tool exceeded 60 seconds')), 60000);
        function finish(error, code) {
            if (settled) { return; }
            settled = true;
            clearTimeout(timer);
            if (error) { process.kill(); reject(error); }
            else { resolve({ code, output, errors }); }
        }
        function append(text, stderr) {
            if (stderr) { errors += text; } else { output += text; }
            if (Buffer.byteLength(output + errors, 'utf8') > 8 * 1024 * 1024) {
                finish(new Error('Tool output exceeded 8 MB'));
            }
        }
        process.stdout.setEncoding('utf8');
        process.stderr.setEncoding('utf8');
        process.stdout.on('data', text => append(text, false));
        process.stderr.on('data', text => append(text, true));
        process.on('error', error => finish(error));
        process.on('close', code => finish(null, code));
        process.stdin.on('error', () => {});
        process.stdin.end(input);
    });
}

function ensureTrusted(vscode) {
    if (!vscode.workspace.isTrusted) {
        throw new Error('Trust this workspace before running Python tools.');
    }
}

function checkArguments(tool, filePath, profile) {
    return tool === 'refactrail'
        ? ['check', filePath, '--profile', profile, '--output-format', 'json', '--no-cache']
        : ['check', filePath, '--format', 'json'];
}

function generalArguments(command, filePath, lineLength = null, bracketStyle = null) {
    const width = lineLength === null ? [] : ['--line-length', String(lineLength)];
    if (bracketStyle !== null && lineLength !== null) {
        if (!['own-line', 'hug'].includes(bracketStyle)) {
            throw new Error('Bracket style must be "own-line" or "hug".');
        }
        width.push('--bracket-style', bracketStyle);
    }
    if (command === 'lint') {
        return ['lint', filePath, '--output-format', 'json', '--no-cache'];
    }
    if (command === 'formatPreview') {
        return ['format', filePath, '--diff', ...width];
    }
    if (command === 'formatWrite') {
        return ['format', filePath, '--write', ...width];
    }
    if (command === 'scope' || command === 'index') { return [command, filePath]; }
    throw new Error('Unsupported general command');
}

function getUtf16Column(lineText, oneBasedColumn) {
    return Array.from(lineText).slice(0, Math.max(0, oneBasedColumn - 1)).join('').length;
}

module.exports = { runPython, ensureTrusted, checkArguments, generalArguments, getUtf16Column };


function isSupportedInput(command, filePath) {
    const extension = require('node:path').extname(filePath).toLowerCase();
    return ['.py', '.pyi'].includes(extension)
        || (extension === '.ipynb' && ['formatPreview', 'formatWrite'].includes(command))
        || command === 'index';
}
module.exports.isSupportedInput = isSupportedInput;
