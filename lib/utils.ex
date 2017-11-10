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

  defmodule OpenGraph do
    def format_og(data) do
      data |> Enum.map(fn {name, content} -> "<meta name=\"#{name}\" content=\"#{content}\">" end)
           |> Enum.join
    end

    def generate_opengraph(_, state, "image") do
      format_og(
        get_default_opengraph(state) ++ [
          {"twitter:card", "summary_large_image"},
          {"og:image", state.redirection},
        ])
    end

    def generate_opengraph(_, state, "video") do
      format_og(
        get_default_opengraph(state) ++ [
          {"og:video", state.redirection},
          {"og:video:type", state.redirection |> MIME.from_path},
        ])
    end

    def generate_opengraph(_, state, "audio") do
      format_og(
        get_default_opengraph(state) ++ [
          {"og:audio", state.redirection},
          {"og:audio:type", state.redirection |> MIME.from_path},
        ])
    end

    def get_default_opengraph(state) do
      [
        {"og:title", state.path},
        {"twitter:site", Application.get_env(:image_ex, :og_twitter)},
        {"twitter:creator", Application.get_env(:image_ex, :og_twitter)},
        {"og:description", Application.get_env(:image_ex, :og_description)}
      ]
    end
  end

  defmodule Crypto do
    @prefix String.pad_trailing(Application.get_env(:image_ex, :block_prefix), 16, "0")
    @aes_block_size 64
    def is_decryptable(data, block_size), do: (block_size - rem(byte_size(data), block_size)) == 0

    def pad(data, block_size) do
      to_add = block_size - rem(byte_size(data), block_size)
      data <> to_string(:string.chars(to_add, to_add))
    end
    
    def unpad(data) do
      to_remove = :binary.last(data)
      :binary.part(data, 0, byte_size(data) - to_remove)
    end

    defp encrypt(data, key, iv), do: {:ok, :crypto.block_encrypt(:aes_cbc256, key, iv, pad(data, 16))}

    defp decrypt(data, key, iv) do
        try do
            {:ok, :crypto.block_decrypt(:aes_cbc256, key, iv, data) |> unpad}
          rescue
            _ in ArgumentError -> {:error, :invalid_data}
            _ -> {:error, :unknown}
          end
    end

    def encrypt_file(file, key, iv) do
      case File.read(file) do
        {:ok, data} ->
          data_hash = :crypto.hash(:sha256, data) |> to_string

          case encrypt(data, key, iv) do
              {:ok, data} ->
                  {:ok, @prefix <> iv <> data_hash <> data}
              err -> err
          end
        err -> {:error, err}
      end
    end

    def decrypt_file(file, key \\ Application.get_env(:image_ex, :aes_key) |> Base.decode16!) do
      case File.read(file) do
        {:ok, data} ->
          case data do
              <<@prefix, iv :: binary-size(16), _ :: binary-size(32), rest :: binary>> ->
                  {:ok, decrypt(rest, key, iv) |> elem(1)}
              _ -> {:error, :invalid_data}
          end
        err -> {:error, err}
      end
    end

    def decrypt_file_stream(file, key \\ Application.get_env(:image_ex, :aes_key) |> Base.decode16!) do
      case ImageEx.Utils.read(file, 0, 64) do
        <<@prefix, iv :: binary-size(16), _ :: binary-size(32)>> ->
          {:ok, decrypt_file_stream(file, key, iv)}
        _ -> {:error, :invalid_data}
      end
    end

    defp decrypt_file_stream(file, key, iv) do
      last_index = round(Float.ceil(File.stat!(file).size / @aes_block_size))
      File.stream!(file, [], @aes_block_size) |> Stream.with_index(1) |> Stream.drop(div(64, @aes_block_size)) |> Stream.transform(iv, fn ({data, index}, vector) ->
        res = :crypto.block_decrypt(:aes_cbc256, key, vector, data)
        res = case index == last_index do
          true -> res |> unpad
          false -> res
        end
        case byte_size(data) - 16 do
          0 -> {[res], data}
          offset -> {[res], :binary.part(data, offset, 16)}
        end
      end)
    end
  end

  def get_base_uri(conn), do: "#{conn.scheme}://#{conn.host}#{port_suffix(conn.scheme,conn.port)}/"
  defp port_suffix(:http,80), do: ""
  defp port_suffix(:https,443), do: ""
  defp port_suffix(_,port), do: ":#{port}"

  def read(file, start, length) do
    {:ok, f} = :file.open(file, [:binary])
    {:ok, data} = :file.pread(f, start, length)
    :file.close(f)
    data
  end
end