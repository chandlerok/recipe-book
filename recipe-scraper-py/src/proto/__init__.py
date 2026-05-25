import sys
from pathlib import Path

_self = Path(__file__).resolve().parent
if str(_self) not in sys.path:
    sys.path.insert(0, str(_self))
