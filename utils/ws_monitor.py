import asyncio
from websockets.asyncio.client import connect

URL = "ws://localhost:8080/ws?hz=30"

async def main():
    async with connect(URL) as ws:
        async for message in ws:
            print(f"\r\033[K{message}", end="", flush=True)

try:
    asyncio.run(main())
except KeyboardInterrupt:
    print()
