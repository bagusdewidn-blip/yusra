"""Modul contoh untuk uji ekstraksi simbol. Tidak dijalankan."""

import os
import json as j, logging.handlers
from typing import Optional
from .models import User
from . import helpers, utils as u
from ..config import settings

MAX_RETRY = 3
TIMEOUT: float = 2.5
logger = logging.getLogger(__name__)
_cache = {}


# komentar biasa, bukan doc
def make_token(user_id: int) -> str:
    """Buat token."""
    return f"t{user_id}"


@dataclass(frozen=True)
class Session(Base):
    """Session login."""

    id: int
    token: str

    class Meta:
        ordering = ["id"]

        def describe(self):
            return "meta"

    def refresh(self) -> bool:
        self.token = make_token(self.id)
        return True

    @property
    def active(self) -> bool:
        return bool(self.token)

    @staticmethod
    async def load(user_id: int) -> Optional["Session"]:
        import pickle

        return pickle.loads(os.environ.get("S", ""))


async def fetch(url):
    return await client.get(url)


if os.name == "nt":

    def platform():
        return "windows"


if __name__ == "__main__":
    print(json.dumps({"ok": True}))
