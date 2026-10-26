# Privacy Policy

Flowsurface does not include telemetry, analytics, advertising, or user accounts, and does not transfer information to other networked systems unless needed to fetch data from data providers or requested by the user.

Flowsurface connects to the third-party data providers enabled in its provider list (all supported providers are enabled by default and can be disabled). These providers receive the requested data type and identifiers, along with the user's IP address and request metadata. Requests go to a supported provider or to a data server configured by the user, which also receives the optional bearer token. Connections to user-configured data servers accept invalid TLS certificates, including self-signed ones, so they should only be used on trusted networks.

The optional content catalog is the only feature that contacts `https://api.flowsurface.com`, and only when the user opens it. Requests carry only the User-Agent `flowsurface/<version>`. The API is hosted on Cloudflare, which may log and retain request metadata, including IP addresses, under its own privacy policy; the API itself serves static files and keeps no record of these requests. Installing content manually avoids this host entirely.

Settings, caches, and logs are stored locally and are not encrypted or uploaded. Proxy credentials and data-server tokens are stored in the system keychain. If a proxy is configured, all application traffic uses it.

Flowsurface does not control the privacy practices of data providers, data servers, proxies, or Cloudflare.
