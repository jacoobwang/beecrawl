import asyncio

from bee_engine.browser import _public_host


def test_browser_dns_policy_blocks_local_and_reserved_addresses() -> None:
    async def scenario() -> None:
        for host in ["localhost", "127.0.0.1", "10.0.0.1", "169.254.169.254", "::1"]:
            assert not await _public_host(host)
        assert await _public_host("1.1.1.1")

    asyncio.run(scenario())
