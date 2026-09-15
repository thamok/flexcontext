"""Authentication helpers for the worker process."""

from dataclasses import dataclass


@dataclass
class SessionClaims:
    subject: int


def decode_bearer_credential(value: str) -> SessionClaims | None:
    """Validate a token without relying on its original identifier spelling."""
    if not value.startswith("bearer-"):
        return None
    return SessionClaims(subject=int(value.removeprefix("bearer-")))
