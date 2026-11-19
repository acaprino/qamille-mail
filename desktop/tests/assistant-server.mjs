// The assistant «Server Qamille» (gateway LLM, wave 5): consent text, provider choice, new consent.
import assert from 'node:assert/strict';
import test from 'node:test';
import { consentText, llmModelName, needsReconsent, providerItems, reconsentText } from '../src/lib/assistant-server.ts';

const external = { model: 'gpt', models: ['gpt'], external: true, label: 'OpenAI', context_tokens: null };
const internal = { model: 'llm-medio', models: ['llm-medio'], external: false, label: 'Server', context_tokens: 8192 };

test('the consent says the provider by name when the server forwards outside', () => {
  assert.equal(consentText(external).it, "Le richieste all'assistente vengono inoltrate a OpenAI, un servizio esterno a questo server.");
  assert.equal(consentText(internal).it, "L'assistente usa un modello che gira su questo server.");
  assert.equal(consentText(null), null);
  assert.equal(consentText(undefined), null);
});

test('«Server Qamille» comes first only when the server has an assistant', () => {
  const ready = providerItems({ kind: 'ready', label: 'Server', external: false });
  assert.equal(ready[0].value, 'QamilleServer');
  assert.deepEqual(ready.filter((item) => item.ownKey).map((item) => item.value), ['Anthropic', 'OpenAiCompatible', 'Ollama']);
  assert.equal(providerItems({ kind: 'consent_needed', label: 'x', external: true })[0].value, 'QamilleServer');
  for (const status of [null, { kind: 'unavailable' }]) {
    const items = providerItems(status);
    assert.equal(items.some((item) => item.value === 'QamilleServer'), false);
    assert.equal(items.some((item) => item.ownKey), true);
  }
});

test('a changed provider asks for a new consent', () => {
  assert.equal(needsReconsent({ kind: 'consent_needed', label: 'Ollama', external: false }), true);
  assert.equal(needsReconsent({ kind: 'ready', label: 'Ollama', external: false }), false);
  assert.equal(needsReconsent(null), false);
  assert.match(reconsentText({ kind: 'consent_needed', label: 'OpenAI', external: true }).it, /OpenAI.*conferma per continuare/);
  assert.equal(reconsentText({ kind: 'ready', label: 'x', external: false }), null);
});

test('the models of the built-in assistant have a readable name', () => {
  assert.equal(llmModelName('llm-medio').it, 'Assistente medio');
  assert.equal(llmModelName('llm-grande').en, 'Assistant large');
  assert.equal(llmModelName('embeddinggemma-2'), null);
});
