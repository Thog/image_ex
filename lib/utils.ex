defmodule Puush do
  def up(key, file_name), do: up(key, file_name, File.read!(file_name))
  def up(key, name, data) do
    boundary = "------#{:crypto.strong_rand_bytes(128) |> Base.encode16}"
    payload = multipart_payload(boundary, [{"z", "waifu"}, {"k", key}], [{"f", name, data}])
    case :httpc.request(:post, {'https://puush.me/api/up', [], 'multipart/form-data; boundary=#{boundary}', payload}, [], []) do
      {:ok, {{_,200,_},_,body}} ->
        case body |> to_string |> String.split(",") do
          ["0", url,_,_] -> {:ok, url}
          data -> {:error, List.first(data)}
        end
      _ -> {:error, nil}
    end
  end

  def multipart_payload(boundary, fields, files) do
    fieldsPart = Enum.map(fields, fn ({key, value}) ->
      [
        "--#{boundary}",
        "Content-Disposition: form-data; name=\"#{key}\"\r\n",
        "#{value}"
      ]
    end) |> List.flatten

    filesPart = Enum.map(files, fn ({key, file_name, file_content}) ->
      [
        "--#{boundary}",
        "Content-Disposition: form-data; name=\"#{key}\"; filename=\"#{file_name}\"",
        "Content-Type: application/octet-stream\r\n",
        "#{file_content}"
      ]
    end) |> List.flatten
    (fieldsPart ++ filesPart ++ ["--#{boundary}--"])|> Enum.join("\r\n")
  end
end

defmodule ImageEx.Utils do
  def get_base_uri(conn) do
    "#{conn.scheme}://#{conn.host}#{port_suffix(conn.scheme,conn.port)}/"
  end

  def format_og(data) do
    data |> Enum.map(fn {name, content} -> "<meta name=\"#{name}\" content=\"#{content}\">" end)
         |> Enum.join
  end

  defp port_suffix(:http,80), do: ""
  defp port_suffix(:https,443), do: ""
  defp port_suffix(_,port), do: ":#{port}"
end