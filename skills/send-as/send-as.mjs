export function finalizeSend(inputs) {
  const plan = object(inputs.send_plan);
  const apply = object(inputs.apply_result);
  const readback = object(inputs.readback_result);
  const delivery = object(inputs.delivery);
  const provider = text(delivery.provider);
  const target = text(delivery.target);
  const operation = text(delivery.operation);
  const errors = [];

  if (text(plan.decision) !== "ready") errors.push("send plan was not ready");
  if (text(apply.status) !== "success") errors.push("provider mutation did not succeed");
  if (text(readback.status) !== "success") errors.push("provider readback did not succeed");
  if (text(apply.provider) !== provider || text(readback.provider) !== provider) {
    errors.push("provider identity changed across apply and readback");
  }
  if (text(apply.target) !== target || text(readback.target) !== target) {
    errors.push("delivery target changed across apply and readback");
  }
  if (text(apply.operation) !== operation) errors.push("provider mutation operation drifted");

  const complete = errors.length === 0;
  return {
    send_result: {
      schema: "runx.send_as.result.v1",
      status: complete ? "sent" : "failed",
      outcome: complete ? "completed" : "failed",
      provider,
      target,
      operation,
      content_digest: text(plan.content?.digest),
      operation_id: text(apply.operation_id),
      readback_ref: text(readback.readback_ref),
      idempotency_key: text(apply.idempotency_key),
      evidence: {
        mutation_readback_ref: text(apply.readback_ref),
        verification_readback_ref: text(readback.readback_ref),
      },
      errors,
    },
  };
}

function object(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function text(value) {
  return typeof value === "string" ? value : "";
}
