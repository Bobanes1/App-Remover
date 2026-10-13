"""Extra knowledge for apps that are known to scatter themselves around."""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import List, Optional

from .model import Program


@dataclass
class Profile:
    name: str
    match: str  # regex matched against the display name
    processes: List[str] = field(default_factory=list)
    services: List[str] = field(default_factory=list)
    paths: List[str] = field(default_factory=list)  # may contain %VARS%
    note: str = ""


PROFILES = [
    Profile(
        name="Adobe Creative Cloud",
        match=r"^adobe creative cloud( desktop)?$",
        processes=[
            "Creative Cloud.exe", "Adobe Desktop Service.exe", "CCXProcess.exe",
            "CCLibrary.exe", "AdobeIPCBroker.exe", "Adobe CEF Helper.exe",
            "Creative Cloud Helper.exe", "AdobeUpdateService.exe", "armsvc.exe",
            "AGMService.exe", "AGSService.exe",
        ],
        services=[
            "AdobeUpdateService", "AdobeARMservice", "AGMService", "AGSService",
            "Adobe Genuine Monitor Service", "Adobe Genuine Software Integrity Service",
        ],
        paths=[
            r"%ProgramFiles(x86)%\Adobe\Adobe Creative Cloud",
            r"%ProgramFiles%\Adobe\Adobe Creative Cloud",
            r"%ProgramFiles%\Adobe\Adobe Creative Cloud Experience",
            r"%ProgramFiles(x86)%\Adobe\Adobe Creative Cloud Experience",
            r"%ProgramData%\Adobe\Creative Cloud",
            r"%APPDATA%\Adobe\Creative Cloud Libraries",
            r"%LOCALAPPDATA%\Adobe\Creative Cloud Libraries",
            r"%LOCALAPPDATA%\Adobe\CoreSync",
            r"%LOCALAPPDATA%\Adobe\OOBE",
        ],
        note="Adobe's own uninstaller is usually missing; removing manually instead.",
    ),
]


def find_profile(program: Program) -> Optional[Profile]:
    for p in PROFILES:
        if re.match(p.match, program.name.strip(), re.I):
            return p
    return None
