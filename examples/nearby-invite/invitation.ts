export interface Invitation {
  version: 1;
  invitationId: string;
  expiresAtMs: number;
  preview: { title: string };
}

export type InvitationDecision =
  | { kind: "accepted"; invitationId: string }
  | { kind: "declined"; invitationId: string; reason?: string }
  | { kind: "expired"; invitationId: string };

export class InvitationInbox {
  readonly #seen = new Map<string, InvitationDecision>();

  constructor(readonly maxDecisions = 256) {
    if (!Number.isSafeInteger(maxDecisions) || maxDecisions <= 0) {
      throw new Error("decision capacity must be positive");
    }
  }

  receive(invitation: Invitation, nowMs: number): InvitationDecision | null {
    validateInvitation(invitation);
    const previous = this.#seen.get(invitation.invitationId);
    if (previous) return previous;
    if (invitation.expiresAtMs <= nowMs) {
      const decision: InvitationDecision = {
        kind: "expired",
        invitationId: invitation.invitationId,
      };
      this.#remember(invitation.invitationId, decision);
      return decision;
    }
    return null;
  }

  decide(invitationId: string, accepted: boolean): InvitationDecision {
    const previous = this.#seen.get(invitationId);
    if (previous) return previous;
    const decision: InvitationDecision = accepted
      ? { kind: "accepted", invitationId }
      : { kind: "declined", invitationId };
    this.#remember(invitationId, decision);
    return decision;
  }

  #remember(invitationId: string, decision: InvitationDecision): void {
    this.#seen.set(invitationId, decision);
    while (this.#seen.size > this.maxDecisions) {
      const oldest = this.#seen.keys().next();
      if (oldest.done === true) break;
      this.#seen.delete(oldest.value);
    }
  }
}

function validateInvitation(value: Invitation): void {
  if (value.version !== 1) throw new Error("unsupported invitation version");
  if (value.invitationId.length < 8 || value.invitationId.length > 128) {
    throw new Error("invalid invitation ID");
  }
  if (value.preview.title.length > 80) throw new Error("preview title is too long");
}

