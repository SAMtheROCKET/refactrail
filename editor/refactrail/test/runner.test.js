'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const { runPython, ensureTrusted, checkArguments, generalArguments, getUtf16Column } = require('../runner');

test('untrusted workspaces cannot run Python', () => {
    assert.throws(() => ensureTrusted({ workspace: { isTrusted: false } }), /Trust/);
    ensureTrusted({ workspace: { isTrusted: true } });
});

test('arguments preserve paths containing spaces and shell characters', () => {
    const source = 'C:\\example space\\$(bad);file.py';
    assert.deepEqual(checkArguments('funcloom', source),
        ['check', source, '--format', 'json']);
    assert.deepEqual(checkArguments('refactrail', source, 'strict'),
        ['check', source, '--profile', 'strict', '--output-format', 'json', '--no-cache']);
});

test('runner uses isolated Python, argument arrays and no shell', async () => {
    let actual;
    const result = await runPython('C:\\python path\\python.exe', 'funcloom',
        ['snippet', '-'], 'amount = 1', (executable, args, options) => {
            actual = { executable, args, options };
            const process = new EventEmitter();
            process.stdout = new EventEmitter(); process.stdout.setEncoding = () => {};
            process.stderr = new EventEmitter(); process.stderr.setEncoding = () => {};
            process.stdin = new EventEmitter();
            process.stdin.end = input => {
                assert.equal(input, 'amount = 1');
                queueMicrotask(() => {
                    process.stdout.emit('data', '{"status":"candidate_for_review"}');
                    process.emit('close', 0);
                });
            };
            process.kill = () => {};
            return process;
        });
    assert.equal(actual.options.shell, false);
    assert.equal(actual.options.windowsHide, true);
    assert.deepEqual(actual.args, ['-I', '-m', 'funcloom', 'snippet', '-']);
    assert.equal(result.code, 0);
    assert.equal(JSON.parse(result.output).status, 'candidate_for_review');
});

test('unknown module names are refused', () => {
    assert.throws(() => runPython('python', 'untrusted', []), /Unsupported/);
});

test('general commands preserve paths and only explicit formatWrite writes', () => {
    const source = 'C:\\sample space\\source.py';
    assert.deepEqual(generalArguments('lint', source),
        ['lint', source, '--output-format', 'json', '--no-cache']);
    assert.deepEqual(generalArguments('formatPreview', source),
        ['format', source, '--diff']);
    assert.deepEqual(generalArguments('formatWrite', source),
        ['format', source, '--write']);
    assert.throws(() => generalArguments('unknown', source), /Unsupported/);
});

test('diagnostic Unicode code-point columns map to editor UTF-16 positions', () => {
    assert.equal(getUtf16Column('a\u{1F600}b', 3), 3);
    assert.equal(getUtf16Column('a\u{1F600}b', 2), 1);
    assert.equal(getUtf16Column('ordinary', 4), 3);
});


test('notebooks are accepted only for formatting and widths stay explicit', () => {
    const { isSupportedInput } = require('../runner');
    assert.equal(isSupportedInput('formatPreview', 'analysis.ipynb'), true);
    assert.equal(isSupportedInput('formatWrite', 'analysis.ipynb'), true);
    assert.equal(isSupportedInput('lint', 'analysis.ipynb'), false);
    assert.equal(isSupportedInput('scope', 'library.pyi'), true);
    assert.deepEqual(generalArguments('formatPreview', 'analysis.ipynb', 79),
        ['format', 'analysis.ipynb', '--diff', '--line-length', '79']);
    assert.deepEqual(generalArguments('scope', 'source.py'), ['scope', 'source.py']);
    assert.deepEqual(generalArguments('index', 'source root'), ['index', 'source root']);
});

test('bracket style reaches formatting and is validated', () => {
    assert.deepEqual(generalArguments('formatWrite', 'a.py', 79, 'hug'),
        ['format', 'a.py', '--write', '--line-length', '79', '--bracket-style', 'hug']);
    assert.deepEqual(generalArguments('formatPreview', 'a.py', 79, null),
        ['format', 'a.py', '--diff', '--line-length', '79']);
    assert.throws(() => generalArguments('formatWrite', 'a.py', 79, 'tight'), /Bracket style/);
});
