"""Provider URLs protect credentials and payloads outside explicit loopback use."""

from urllib.parse import urlsplit

# The names and addresses plain HTTP is admitted on when the configuration asks
# for localhost development: a provider URL, the hub's own listener and a client.
LOOPBACK_HOSTS = frozenset({"localhost", "127.0.0.1", "::1"})


def protected_url(value: str, *, allow_insecure_localhost: bool):
    url = urlsplit(value)
    if (
        not url.hostname
        or url.username is not None
        or url.password is not None
        or url.query
        or url.fragment
        or value != value.strip()
    ):
        raise ValueError("A plain URL must not contain credentials, query or fragment")
    if url.scheme != "https" and not (
        url.scheme == "http" and allow_insecure_localhost and url.hostname in LOOPBACK_HOSTS
    ):
        raise ValueError("URL requires HTTPS, except explicit localhost development")
    _ = url.port
    return url
