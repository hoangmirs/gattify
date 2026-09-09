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

  receive(invitation: Invitation, nowMs: number): InvitationDecision | null {
    const previous = this.#seen.get(invitation.invitationId);
    if (previous) return previous;
    validateInvitation(invitation);
    if (invitation.expiresAtMs <= nowMs) {
      const decision: InvitationDecision = {
        kind: "expired",
        invitationId: invitation.invitationId,
      };
      this.#seen.set(invitation.invitationId, decision);
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
    this.#seen.set(invitationId, decision);
    return decision;
  }
}

function validateInvitation(value: Invitation): void {
  if (value.version !== 1) throw new Error("unsupported invitation version");
  if (value.invitationId.length < 8 || value.invitationId.length > 128) {
    throw new Error("invalid invitation ID");
  }
  if (value.preview.title.length > 80) throw new Error("preview title is too long");
}

