# Drop-in equivalent of nixconfig's blaz.nix for Bouedig. Add the flake
# input to your nixconfig,
#
#   inputs.bouedig.url = "github:MathieuMoalic/bouedig";
#
# then import this module from your selfhosted host and add the two sops
# secrets (`bouedig/password`, `bouedig/llm-api-key`).
{
  flake.nixosModules.bouedig-deploy = {
    config,
    pkgs,
    inputs,
    ...
  }: let
    url = "bouedig.matmoa.eu";
    port = 10025;

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
    services.bouedig = {
      enable = true;
      # `prebuilt` avoids compiling the Rust workspace on the server; it
      # needs `prebuiltHash` in the flake set to the latest release hash
      # (CI prints it). Use `packages.bouedig` to build from source instead.
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
