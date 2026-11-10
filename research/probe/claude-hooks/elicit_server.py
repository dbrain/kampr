from mcp.server.fastmcp import FastMCP, Context
from pydantic import BaseModel

app = FastMCP("probe")


class Pick(BaseModel):
    colour: str
    count: int


@app.tool()
async def ask_colour(ctx: Context) -> str:
    """Ask the user for a colour and a count."""
    r = await ctx.elicit(message="Which colour, and how many?", schema=Pick)
    return f"action={r.action} data={getattr(r, 'data', None)}"


app.run()
