"""A fake Hermes panel: the REST routes the hub uses for a `hermes` agent's
SOUL.md and skills, with the shapes of Hermes's `web_routers/profiles.py` and
`web_routers/skills.py`. It keeps one profile's files in memory, records each
call, and can be told to refuse a route with a status. No real Hermes runs.
"""

from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse

TOKEN = "credencial-sintetica-del-panel"


class FakeHermesPanel:
    def __init__(self, profile: str):
        self.app = FastAPI()
        self.profile = profile
        self.soul: str | None = None
        # name -> {description, content, enabled}
        self.skills: dict[str, dict] = {}
        self.calls: list[tuple[str, str, dict]] = []
        # "METHOD /route" -> status to answer instead.
        self.refuse: dict[str, int] = {}

        def admit(request: Request, route: str, body: dict | None = None):
            self.calls.append((request.method, route, {**request.query_params, **(body or {})}))
            if request.headers.get("x-hermes-session-token") != TOKEN:
                raise HTTPException(401, "Unauthorized")
            status = self.refuse.get(f"{request.method} {route}")
            if status is not None:
                raise HTTPException(status, "Refused by the fake panel")

        def profile_of(name: str) -> None:
            if name != self.profile:
                raise HTTPException(404, f"Profile '{name}' not found")

        @self.app.get("/api/profiles/{name}/soul")
        async def get_soul(name: str, request: Request):
            admit(request, "/api/profiles/{name}/soul")
            profile_of(name)
            if self.soul is None:
                return {"content": "", "exists": False}
            return {"content": self.soul, "exists": True}

        @self.app.put("/api/profiles/{name}/soul")
        async def put_soul(name: str, request: Request):
            body = await request.json()
            admit(request, "/api/profiles/{name}/soul", body)
            profile_of(name)
            self.soul = body["content"]
            return {"ok": True}

        @self.app.get("/api/skills")
        async def list_skills(request: Request):
            admit(request, "/api/skills")
            return [
                {
                    "name": name,
                    "description": skill["description"],
                    "category": None,
                    "enabled": skill["enabled"],
                    "usage": 0,
                    "provenance": "agent",
                }
                for name, skill in self.skills.items()
            ]

        @self.app.get("/api/skills/content")
        async def skill_content(request: Request):
            admit(request, "/api/skills/content")
            name = request.query_params["name"]
            if name not in self.skills:
                raise HTTPException(404, f"Skill '{name}' not found.")
            return {"name": name, "content": self.skills[name]["content"], "path": "/invented"}

        @self.app.post("/api/skills")
        async def create_skill(request: Request):
            body = await request.json()
            admit(request, "/api/skills", body)
            if body["name"] in self.skills:
                return JSONResponse({"detail": "exists"}, status_code=400)
            self.skills[body["name"]] = {
                "description": description_of(body["content"]),
                "content": body["content"],
                "enabled": True,
            }
            return {"success": True}

        @self.app.put("/api/skills/content")
        async def replace_skill(request: Request):
            body = await request.json()
            admit(request, "/api/skills/content", body)
            if body["name"] not in self.skills:
                raise HTTPException(404, "Skill not found")
            self.skills[body["name"]].update(
                content=body["content"], description=description_of(body["content"])
            )
            return {"success": True}

        @self.app.put("/api/skills/toggle")
        async def toggle_skill(request: Request):
            body = await request.json()
            admit(request, "/api/skills/toggle", body)
            if body["name"] in self.skills:
                self.skills[body["name"]]["enabled"] = body["enabled"]
            return {"ok": True, "name": body["name"], "enabled": body["enabled"]}


def description_of(content: str) -> str:
    for line in content.splitlines():
        if line.startswith("description:"):
            return line.removeprefix("description:").strip()
    return ""
