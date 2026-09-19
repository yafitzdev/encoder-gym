// Deterministic Pi wire fixture. Uses the real CLI's reserved-turn transport;
// no production flag or application code substitutes Agent captions.
import readline from 'node:readline';
import { appendFileSync, writeFileSync } from 'node:fs';
const send = value => process.stdout.write(JSON.stringify(value) + '\n');
send({ type: 'ready', protocolVersion: 1, piPackageVersion: '0.84.4' });
let runId;
readline.createInterface({ input: process.stdin }).on('line', line => {
  const message = JSON.parse(line);
  if (message.type === 'start') {
    const request = message.request;
    runId = request.runId;
    if (request.model !== 'pinned-agent' || !request.openaiCompatible) throw Error('Wrong pinned provider');
    if (JSON.stringify(request).includes('NEVER_DISCLOSE_HOLDOUT')) throw Error('Protected evidence leaked');
    const input = JSON.parse(request.initialPrompt);
    appendFileSync(process.env.AGENT_FIXTURE_CALLS, JSON.stringify({model: request.model, runId, iteration: input.scope.iteration, evidence: input.scope.developmentEvidenceFingerprint}) + '\n');
    const turns = input.previousTurns.filter(turn => !turn.interrupted);
    const later = input.scope.iteration > 1;
    send({ type: 'event', event: {type: 'turn_started', runId, sequence: 1} });
    if (process.env.AGENT_FIXTURE_HOLD && turns.length === 1) {
      writeFileSync(process.env.AGENT_FIXTURE_HOLD, 'waiting for stop');
      return;
    }
    let name, args;
    if (input.scope.analysisProtocol === 2) {
      const clusters = turns[0]?.tools[0]?.result?.items ?? [];
      const cluster = clusters.find(item => item.content.cluster?.dimension === 'expected_capability' && item.content.cluster?.value === 'search') ?? clusters[0];
      if (!turns.length) {
        name = 'inspect_dataset_landscape'; args = {offset: 0, limit: 20};
      } else if (turns.length === 1) {
        if (!cluster || cluster.content.kind !== 'dataset_cluster') throw Error('No dataset landscape cluster');
        if (later) {
          const examples = cluster.content.development?.flatMap(value => value.sampledFailureDiagnostics?.examples ?? []) ?? [];
          if (!examples.some(example => example.expectedRank === 3 && example.questionPreview.includes('candidate regression'))) throw Error('Second iteration did not receive candidate evaluation');
        }
        name = 'inspect_dataset_clusters'; args = {clusterIds: [cluster.id], examplesPerCluster: 4};
      } else {
        const rows = turns[1].tools[0].result.items;
        if (!cluster || !rows.length) throw Error('No real cluster evidence or inspected row');
        const row = rows.find(item => item.content.question.includes(later ? 'Retain' : 'Search')) ?? rows[0];
        if (process.env.AGENT_FIXTURE_LOOP === 'no_change_first' || (later && (process.env.AGENT_FIXTURE_LOOP === 'no_change' || input.scope.iteration === 3))) {
          name = 'propose_dataset_edits'; args = {summary: 'The candidate regression does not justify another dataset change.', stop: true, removals: [], additions: []};
        } else {
          name = 'propose_dataset_edits';
          args = {summary: 'Shift search coverage by one row to address the weak search cluster.', stop: false,
            removals: [{rowId: row.id, reason: 'Ambiguous wording conflicts with the weak inspected cluster.', evidenceIds: [cluster.id]}],
            additions: [{templateRowId: row.id, instruction: 'Add one specific search request for the weak search cluster.', count: 1, evidenceIds: [cluster.id]}]};
          if (later) {
            if (input.scope.maximumRowChanges !== 6) throw Error('Cumulative edit budget was reset');
            args = {summary: 'The candidate regression points to conflicting search coverage; remove one inspected row.', stop: false,
              removals: [{rowId: row.id, reason: 'Changed candidate evidence identifies conflicting coverage.', evidenceIds: [cluster.id]}], additions: []};
          }
        }
      }
    } else if (!turns.length) {
      name = 'inspect_development_failures'; args = {offset: 0, limit: 20};
    } else if (turns.length === 1) {
      if (later) {
        const evidence = turns[0].tools[0].result.items;
        const failure = evidence.find(item => item.content.evidenceScope === 'retrieval_failure_sample');
        if (failure?.content.expectedRank !== 3 || !failure.content.question.includes('candidate regression')) throw Error('Second iteration did not receive candidate evaluation');
        if (!evidence.some(item => item.content.kind === 'development_summary' && item.content.assessments)) throw Error('Previous metrics and decision missing');
      }
      if (process.env.AGENT_FIXTURE_LOOP === 'no_change_first' || (later && (process.env.AGENT_FIXTURE_LOOP === 'no_change' || input.scope.iteration === 3))) {
        name = 'propose_dataset_edits'; args = {summary: 'The candidate regression does not justify another dataset change.', stop: true, removals: [], additions: []};
      } else {
        name = 'inspect_training_rows'; args = {offset: 0, limit: 20, query: later ? 'Retain' : 'Search'};
      }
    } else {
      const evidence = turns[0].tools[0].result.items.find(item => item.content.evidenceScope === 'retrieval_failure_sample');
      if (!turns[0].tools[0].result.items.some(item => item.content.kind === 'development_summary')) throw Error('Metrics are not on the first inspection page');
      const rows = turns[1].tools[0].result.items;
      if (!evidence || !rows.length) throw Error('No real inspected evidence');
      if (evidence.content.evidenceScope !== 'retrieval_failure_sample' || evidence.content.sampleLimit !== 50) throw Error('Diagnostic sample was not labeled');
      name = 'propose_dataset_edits';
      args = {summary: 'Replace the ambiguous search example and add the missing specific search request.', stop: false,
        removals: [{rowId: rows[0].id, reason: 'Ambiguous wording conflicts with the inspected retrieval failure.', evidenceIds: [evidence.id]}],
        additions: [{templateRowId: rows[0].id, instruction: 'Cover the specific search request from the failure.', count: 1, evidenceIds: [evidence.id]}]};
      if (later) {
        if (input.scope.maximumRowChanges !== 6) throw Error('Cumulative edit budget was reset');
        args = {summary: 'The candidate regression points to conflicting retain coverage; remove that inspected row.', stop: false,
          removals: [{rowId: rows[0].id, reason: 'Changed candidate evidence identifies conflicting retain coverage.', evidenceIds: [evidence.id]}], additions: []};
      }
    }
    if (process.env.AGENT_FIXTURE_LOOP === 'canary_rejected' && name === 'propose_dataset_edits') args.additions[0].count = 17;
    send({type: 'tool_request', runId, callId: 'fixture-tool', name, arguments: args});
  } else if (message.type === 'tool_result') {
    send({type: 'event', event: {type: 'turn_completed', runId, sequence: 1, inputTokens: 150, outputTokens: 70, costMicrousd: 0}});
    send({type: 'completed', runId});
  } else if (message.type === 'tool_error') {
    send({type: 'failed', runId, message: message.message});
  } else if (message.type === 'cancel') process.exit(0);
});
