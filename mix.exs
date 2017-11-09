defmodule ImageEx.Mixfile do
  use Mix.Project

  def project do
    [app: :image_ex,
     version: "0.1.1",
     elixir: "~> 1.5",
     build_embedded: Mix.env == :prod,
     start_permanent: Mix.env == :prod,
     deps: deps()]
  end

  def application do
    [applications: [:logger, :ewebmachine, :cowboy, :poison, :timex],
     mod: {ImageEx.App, []} ]
  end

  defp deps do
    [
      {:poison, "~> 3.1"},
      {:ewebmachine, git: "https://github.com/kbrw/ewebmachine.git", branch: "master"},
      {:cowboy, "~> 1.1"},
      {:timex, "~> 3.1"},
      {:plug, "~> 1.4", override: true},
      {:distillery, "~> 1.5"}
    ]
  end
end
