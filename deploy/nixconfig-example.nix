# bouedig.nix — drop this in your nixconfig next to blaz.nix
# (e.g. nixos/selfhosted/bouedig.nix). Wiring needed elsewhere:
#
#   1. flake input:  bouedig.url = "github:MathieuMoalic/bouedig";
#   2. self-hosted.nix imports:
#        inputs.bouedig.nixosModules.bouedig-service
#        bouedig            # (via `with self.nixosModules`)
#   3. sops secrets in secrets.yaml (keys `bouedig/password` and
#      `bouedig/llm-api-key`), e.g. `sops secrets.yaml`:
#
#        bouedig:
#            password: <your household password>
#            llm-api-key: sk-or-v1-...
#
# Recipe browsing is public; everything else needs the password.
{
  flake.nixosModules.bouedig = {
    config,
    pkgs,
    inputs,
    ...
  }: let
    url = "bouedig.matmoa.eu";
    port = 10001;

    s = config.sops.secrets;
    passwordFile = s."bouedig/password".path;
    llmApiKeyFile = s."bouedig/llm-api-key".path;
  in {
    sops.secrets = {
      "bouedig/password" = {
        owner = "bouedig";
        group = "bouedig";
        mode = "0400";
      };
      "bouedig/llm-api-key" = {
        owner = "bouedig";
        group = "bouedig";
        mode = "0400";
      };
    };
    users.users.mat.extraGroups = ["bouedig"];
    users.users.bouedig.homeMode = "0750";
    services.bouedig = {
      enable = true;
      package = inputs.bouedig.packages.${pkgs.stdenv.hostPlatform.system}.prebuilt;
      bindAddr = "127.0.0.1:${toString port}";

      inherit
        passwordFile
        llmApiKeyFile
        ;
    };

    services.caddy.virtualHosts.${url}.extraConfig = ''
      reverse_proxy 127.0.0.1:${toString port}
    '';
  };
}
