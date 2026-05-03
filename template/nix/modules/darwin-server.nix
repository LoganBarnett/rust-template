# Darwin (macOS/launchd) module for the rust-template-server service.
# Thin wrapper around the foundation's mkDarwinService helper.
# See mkNixosService for the Linux/systemd equivalent.
#
# Minimal usage (defaults to Unix domain socket):
#
#   inputs.rust-template.darwinModules.server
#
#   services.rust-template-server = {
#     enable = true;
#   };
#
# To use TCP instead:
#
#   services.rust-template-server = {
#     enable = true;
#     socket = null;
#     port   = 8080;
#   };
#
# To enable health checking (requires a reachable health endpoint):
#
#   services.rust-template-server = {
#     enable = true;
#     healthCheck.enable = true;
#     healthCheck.url = "http://127.0.0.1:3000/health";
#   };
{
  self,
  foundation,
}:
  foundation.lib.mkDarwinService {
    name = "rust-template-server";
    inherit self;
  }
