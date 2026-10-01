/**
 * Wire-format test: stands up a stub HTTP server, points each provider at it,
 * and checks (a) the request body we send and (b) that we parse a real SSE
 * stream into the right StreamChunks.
 */
import { createServer } from 'node:http';
import assert from 'node:assert';
import { AnthropicProvider, OpenAIProvider, GeminiProvider } from '../src/providers/index.ts';

let captured = null;
let responder = null;

const server = createServer((req, res) => {
  let body = '';
  req.on('data', (c) => (body += c));
  req.on('end', () => {
    captured = { url: req.url, headers: req.headers, body: body ? JSON.parse(body) : null };
    responder(res);
  });
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

const sse = (events) => (res) => {
  res.writeHead(200, { 'content-type': 'text/event-stream' });
  for (const e of events) res.write(`event: ${e.type}\ndata: ${JSON.stringify(e)}\n\n`);
  res.end();
};

async function collect(gen) {
  const out = [];
  for await (const c of gen) out.push(c);
  return out;
}

const HISTORY = [
  { role: 'system', content: 'You are a bot.' },
  { role: 'user', content: 'Book a flight to Bangalore' },
  { role: 'assistant', content: [
      { type: 'text', text: 'Searching.' },
      { type: 'tool_call', id: 'call_1', name: 'search_flights', input: { to: 'BLR' } },
  ]},
  { role: 'user', content: [{ type: 'tool_result', toolCallId: 'call_1', content: '3 flights found' }] },
];
const TOOLS = [{
  name: 'search_flights',
  description: 'Search flights',
  parameters: { type: 'object', properties: { to: { type: 'string' } }, required: ['to'] },
}];

// ---------------------------------------------------------------- Anthropic
responder = sse([
  { type: 'message_start', message: { id: 'msg_1', type: 'message', role: 'assistant', model: 'claude-opus-5', content: [], stop_reason: null, usage: { input_tokens: 42, output_tokens: 0 } } },
  { type: 'content_block_start', index: 0, content_block: { type: 'thinking', thinking: '', signature: '' } },
  { type: 'content_block_delta', index: 0, delta: { type: 'thinking_delta', thinking: 'Weighing options' } },
  { type: 'content_block_stop', index: 0 },
  { type: 'content_block_start', index: 1, content_block: { type: 'text', text: '' } },
  { type: 'content_block_delta', index: 1, delta: { type: 'text_delta', text: 'Cheapest is ' } },
  { type: 'content_block_delta', index: 1, delta: { type: 'text_delta', text: 'IX-2841.' } },
  { type: 'content_block_stop', index: 1 },
  { type: 'content_block_start', index: 2, content_block: { type: 'tool_use', id: 'toolu_9', name: 'search_flights', input: {} } },
  { type: 'content_block_delta', index: 2, delta: { type: 'input_json_delta', partial_json: '{"to":"BLR"}' } },
  { type: 'content_block_stop', index: 2 },
  { type: 'message_delta', delta: { stop_reason: 'tool_use' }, usage: { output_tokens: 17 } },
  { type: 'message_stop' },
]);

const aChunks = await collect(new AnthropicProvider('sk-ant-test', { baseURL: base }).chat(HISTORY, { tools: TOOLS }));
const aBody = captured.body;

assert.equal(aBody.model, 'claude-opus-5');
assert.deepEqual(aBody.thinking, { type: 'adaptive', display: 'summarized' });
assert.equal(aBody.fallbacks, 'default');
assert.equal(aBody.system, 'You are a bot.', 'system must be hoisted out of messages');
assert.ok(!('temperature' in aBody) && !('top_p' in aBody), 'sampling params are 400s on Opus 5');
assert.ok(!aBody.messages.some((m) => m.role === 'system'));
assert.equal(aBody.tools[0].input_schema.type, 'object', 'Anthropic uses input_schema');
assert.equal(aBody.messages[1].content[1].type, 'tool_use');
assert.equal(aBody.messages[2].content[0].type, 'tool_result');
assert.equal(aBody.messages[2].content[0].tool_use_id, 'call_1');
assert.equal(captured.headers['anthropic-beta'], 'server-side-fallback-2026-07-01');
console.log('anthropic request shape: OK');

assert.deepEqual(aChunks.filter((c) => c.type === 'thinking_delta').map((c) => c.text), ['Weighing options']);
assert.equal(aChunks.filter((c) => c.type === 'text_delta').map((c) => c.text).join(''), 'Cheapest is IX-2841.');
const aTool = aChunks.find((c) => c.type === 'tool_call');
assert.deepEqual(aTool.input, { to: 'BLR' }, 'input_json_delta must be assembled');
const aDone = aChunks.at(-1);
assert.equal(aDone.type, 'done');
assert.equal(aDone.stopReason, 'tool_use');
assert.deepEqual(aDone.usage, { inputTokens: 42, outputTokens: 17 });
assert.equal(aDone.message.content.filter((p) => p.type === 'tool_call').length, 1);
console.log('anthropic stream parse: OK');

// ------------------------------------------------------------------- OpenAI
responder = sse([
  { type: 'response.created', response: { id: 'resp_1', model: 'gpt-5.6-sol', status: 'in_progress' } },
  { type: 'response.output_text.delta', delta: 'Cheapest is ' },
  { type: 'response.output_text.delta', delta: 'IX-2841.' },
  { type: 'response.output_item.done', item: { type: 'message', id: 'm1', role: 'assistant', content: [{ type: 'output_text', text: 'Cheapest is IX-2841.' }] } },
  { type: 'response.output_item.done', item: { type: 'function_call', id: 'fc_1', call_id: 'call_xyz', name: 'search_flights', arguments: '{"to":"BLR"}' } },
  { type: 'response.completed', response: { id: 'resp_1', model: 'gpt-5.6-sol', status: 'completed', usage: { input_tokens: 30, output_tokens: 12 } } },
]);

const oChunks = await collect(new OpenAIProvider('sk-test', { baseURL: base }).chat(HISTORY, { tools: TOOLS }));
const oBody = captured.body;

assert.ok(captured.url.includes('/responses'), 'must use Responses API, not chat/completions');
assert.equal(oBody.instructions, 'You are a bot.', 'system -> instructions');
assert.equal(oBody.stream, true);
assert.equal(oBody.tools[0].type, 'function');
assert.equal(oBody.tools[0].parameters.type, 'object');
const fc = oBody.input.find((i) => i.type === 'function_call');
const fo = oBody.input.find((i) => i.type === 'function_call_output');
assert.equal(fc.call_id, 'call_1');
assert.equal(fc.arguments, '{"to":"BLR"}', 'arguments must be a JSON string');
assert.equal(fo.call_id, 'call_1', 'output must reference call_id');
assert.equal(oBody.input.find((i) => i.role === 'assistant').content[0].type, 'output_text');
assert.equal(oBody.input.find((i) => i.role === 'user').content[0].type, 'input_text');
console.log('openai request shape: OK');

assert.equal(oChunks.filter((c) => c.type === 'text_delta').map((c) => c.text).join(''), 'Cheapest is IX-2841.');
const oTool = oChunks.find((c) => c.type === 'tool_call');
assert.equal(oTool.id, 'call_xyz', 'must use call_id, not item.id');
assert.deepEqual(oTool.input, { to: 'BLR' }, 'arguments string must be parsed');
const oDone = oChunks.at(-1);
assert.equal(oDone.stopReason, 'tool_use');
assert.equal(oDone.model, 'gpt-5.6-sol');
assert.deepEqual(oDone.usage, { inputTokens: 30, outputTokens: 12 });
console.log('openai stream parse: OK');

// ------------------------------------------------------------------- Gemini
responder = (res) => {
  res.writeHead(200, { 'content-type': 'text/event-stream' });
  const chunks = [
    { candidates: [{ content: { role: 'model', parts: [{ text: 'Cheapest is ' }] } }], modelVersion: 'gemini-3.6-flash' },
    { candidates: [{ content: { role: 'model', parts: [{ text: 'IX-2841.' }] } }] },
    { candidates: [{ content: { role: 'model', parts: [{ functionCall: { name: 'search_flights', args: { to: 'BLR' } } }] }, finishReason: 'STOP' }],
      usageMetadata: { promptTokenCount: 25, candidatesTokenCount: 9 } },
  ];
  for (const c of chunks) res.write(`data: ${JSON.stringify(c)}\r\n\r\n`);
  res.end();
};

const gChunks = await collect(new GeminiProvider('key-test', { baseURL: base }).chat(HISTORY, { tools: TOOLS }));
const gBody = captured.body;

assert.equal(gBody.systemInstruction.parts[0].text, 'You are a bot.');
assert.ok(gBody.contents.every((c) => c.role === 'user' || c.role === 'model'), 'roles are user/model');
const gCall = gBody.contents.flatMap((c) => c.parts).find((p) => p.functionCall);
const gResp = gBody.contents.flatMap((c) => c.parts).find((p) => p.functionResponse);
assert.equal(gCall.functionCall.name, 'search_flights');
assert.equal(gResp.functionResponse.name, 'search_flights', 'Gemini keys results by NAME, not id');
assert.deepEqual(gResp.functionResponse.response, { result: '3 flights found' });
assert.ok(gBody.tools[0].functionDeclarations[0].name === 'search_flights');
console.log('gemini request shape: OK');

assert.equal(gChunks.filter((c) => c.type === 'text_delta').map((c) => c.text).join(''), 'Cheapest is IX-2841.');
const gTool = gChunks.find((c) => c.type === 'tool_call');
assert.deepEqual(gTool.input, { to: 'BLR' });
assert.ok(gTool.id, 'a synthetic id must be minted when Gemini omits one');
const gDone = gChunks.at(-1);
assert.equal(gDone.stopReason, 'tool_use');
assert.deepEqual(gDone.usage, { inputTokens: 25, outputTokens: 9 });
const gTexts = gDone.message.content.filter((p) => p.type === 'text');
assert.equal(gTexts.length, 1, 'streamed text parts must be coalesced');
assert.equal(gTexts[0].text, 'Cheapest is IX-2841.');
console.log('gemini stream parse: OK');

// ------------------------------------------------------- error path (no throw)
responder = (res) => { res.writeHead(500, { 'content-type': 'application/json' }); res.end('{"error":{"message":"boom"}}'); };
const errChunks = await collect(new AnthropicProvider('sk-ant-test', { baseURL: base }).chat([{ role: 'user', content: 'hi' }]));
assert.equal(errChunks.at(-1).type, 'error', 'API errors must surface as an error chunk, not a throw');
console.log('error path yields error chunk (no throw): OK');

server.close();
console.log('\nALL WIRE TESTS PASSED');
