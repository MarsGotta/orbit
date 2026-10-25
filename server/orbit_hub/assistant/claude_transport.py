"""Keep configured command prefixes when using the pinned official SDK.

The SDK exposes cli_path but no argument prefix. This small extension of its
own transport preserves operator wrappers/flags; all protocol handling and
CLI execution still belong to Anthropic's SDK. Its private method is covered
by the real-SDK fixture and must be checked when upgrading the exact pin.
"""

from dataclasses import replace

from claude_agent_sdk._internal.transport.subprocess_cli import SubprocessCLITransport


class ConfiguredTransport(SubprocessCLITransport):
    def __init__(self, command, options):
        async def empty():
            return
            yield

        self.command = command
        # The SDK normally supplies this on its copy of the options before
        # constructing a transport. A custom transport must receive it too.
        transport_options = replace(options, permission_prompt_tool_name="stdio")
        super().__init__(prompt=empty(), options=transport_options)

    def _build_command(self):
        return [*self.command, *super()._build_command()[1:]]

    async def _check_claude_version(self):
        # The upstream advisory probe executes cli_path alone, dropping any
        # operator prefix, and inherits the hub environment rather than the
        # options' isolated environment. Skip that advisory subprocess; the
        # SDK's normal initialize handshake still checks protocol support.
        return None
