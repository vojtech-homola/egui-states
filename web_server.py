import asyncio
import time
from multiprocessing import Process
from pathlib import Path

from aiohttp import ClientSession, ClientWebSocketResponse, WSMsgType, web

MESSAGE_SIZE_LIMIT = 536870912  # 512 MiB


class WebApp:
    """Aiohttp web application with WebSocket proxy and static file serving."""

    def __init__(self, port: int, port_ws: int) -> None:
        """Initialize the web application."""
        self._port = port
        self._ws_url = f"ws://127.0.0.1:{port_ws}"

        self._current_client: web.WebSocketResponse | None = None  # Track current client connection
        self._current_target: ClientWebSocketResponse | None = None  # Track current target connection
        self._current_session: ClientSession | None = None  # Track current session

        self._app = web.Application()

        # Add WebSocket proxy route
        self._app.router.add_get("/ws", self._websocket_proxy)

        # Add specific static routes
        self._app.router.add_get("/", self._static_handler)

        # Use static directory instead of catch-all pattern
        static_dir = Path(__file__).parent / "_assets/web"
        self._app.router.add_static("/", path=str(static_dir), name="static")

    def start(self):
        """Start the web application."""

        def print_message(*args):
            print(f"Web GUI server running at http://localhost:{self._port}")

        web.run_app(self._app, host="0.0.0.0", port=self._port, print=print_message)

    async def _static_handler(self, request):
        """Serve static files, defaulting to index.html for root."""
        path = request.path

        # Default to index.html for root path
        if path == "/":
            path = "/index.html"

        # Remove leading slash and construct file path
        file_path = Path(__file__).parent / "_assets/web" / path.lstrip("/")

        try:
            if file_path.exists() and file_path.is_file():
                return web.FileResponse(file_path)
            else:
                return web.Response(text="File not found", status=404)
        except Exception as e:
            return web.Response(text=f"Error serving file: {e}", status=500)

    async def _close_current_connection(self):
        """Close the current client and target connections."""
        if self._current_client and not self._current_client.closed:
            await self._current_client.close(code=1000, message=b"New client connected")

        if self._current_target and not self._current_target.closed:
            await self._current_target.close()

        if self._current_session and not self._current_session.closed:
            await self._current_session.close()

    async def _cleanup_connections(self, client_ws, target_ws, session):
        """Clean up all connections."""
        # Clear current connections if they match
        if self._current_client == client_ws:
            self._current_client = None
        if self._current_target == target_ws:
            self._current_target = None
        if self._current_session == session:
            self._current_session = None

        # Close target connection
        if target_ws and not target_ws.closed:
            try:
                await target_ws.close()
            except Exception:
                pass

        # Close session
        if session and not session.closed:
            try:
                await session.close()
            except Exception:
                pass

        # Close client connection
        if client_ws and not client_ws.closed:
            try:
                await client_ws.close(code=1001, message=b"Connection terminated")
            except Exception:
                pass

    async def _websocket_proxy(self, request):
        """Proxy WebSocket connections to target server."""
        # Close any existing connection
        await self._close_current_connection()

        # Upgrade client connection to WebSocket first
        client_ws = web.WebSocketResponse(max_msg_size=MESSAGE_SIZE_LIMIT)
        await client_ws.prepare(request)

        # Now try to connect to target WebSocket server
        session = ClientSession()
        target_ws = None

        try:
            target_ws = await session.ws_connect(self._ws_url, max_msg_size=MESSAGE_SIZE_LIMIT)
        except Exception as e:
            print(f"Failed to connect to target server: {e}")
            await client_ws.send_str(f"Error: Cannot connect to backend server - {e}")
            await client_ws.close()
            await session.close()
            return client_ws

        # Store current connections
        self._current_client = client_ws
        self._current_target = target_ws
        self._current_session = session

        # Event to signal when any side disconnects
        disconnect_event = asyncio.Event()

        async def client_to_target():
            """Forward messages from client to target."""
            try:
                async for msg in client_ws:
                    if msg.type == WSMsgType.BINARY:
                        await target_ws.send_bytes(msg.data)
                    elif msg.type == WSMsgType.TEXT:
                        await target_ws.send_str(msg.data)
                    elif msg.type in (WSMsgType.CLOSE, WSMsgType.ERROR):
                        break
            except Exception as e:
                print(f"Client connection error: {e}")
            finally:
                disconnect_event.set()

        async def target_to_client():
            """Forward messages from target to client."""
            try:
                async for msg in target_ws:
                    if msg.type == WSMsgType.BINARY:
                        await client_ws.send_bytes(msg.data)
                    elif msg.type == WSMsgType.TEXT:
                        await client_ws.send_str(msg.data)
                    elif msg.type in (WSMsgType.CLOSE, WSMsgType.ERROR):
                        break
            except Exception as e:
                print(f"Target server connection error: {e}")
            finally:
                disconnect_event.set()

        async def connection_monitor():
            """Monitor for disconnection and cleanup when either side disconnects."""
            await disconnect_event.wait()
            await self._cleanup_connections(client_ws, target_ws, session)

        # Run proxy and monitor concurrently
        try:
            await asyncio.gather(client_to_target(), target_to_client(), connection_monitor(), return_exceptions=True)
        except Exception as e:
            print(f"Error during proxy operation: {e}")
        finally:
            # Ensure cleanup happens even if something goes wrong
            await self._cleanup_connections(client_ws, target_ws, session)

        return client_ws


class WebAppProcess:
    """Run the web app server in a separate process."""

    def __init__(self, port: int, port_ws: int):
        """Initialize the WebAppProcess."""
        self._process = Process(target=self._run_server)
        self._port = port
        self._port_ws = port_ws

    def _run_server(self):
        web_app = WebApp(port=self._port, port_ws=self._port_ws)
        web_app.start()

    def start(self):
        """Start the web app server process."""
        self._process.start()

    def stop(self):
        """Stop the web app server process."""
        if not self._process.is_alive():
            return

        self._process.terminate()
        while self._process.is_alive():
            time.sleep(0.1)

        self._process.close()


if __name__ == "__main__":
    port = 8080
    web_app = WebApp(port=8080, port_ws=8081)
    web_app.start()
